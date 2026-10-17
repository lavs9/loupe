use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::db::EMBED_DIM;
use crate::embedder::{image_for_embedding, MultimodalEmbedder, MODEL_ID};
use crate::sources::{self, volumes_root};

const LLAMA_TAG: &str = "b11642";
const LLAMA_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/b11642/llama-b11642-bin-macos-arm64.tar.gz";
const LLAMA_TAR_BYTES: u64 = 12_175_015;

const TEXT_FILE: &str = "embeddinggemma-2-Q8_0.gguf";
const VISION_FILE: &str = "mmproj-embeddinggemma-2-Q8_0.gguf";
const TEXT_BYTES: u64 = 309_855_456;
const VISION_BYTES: u64 = 554_821_024;
const TEXT_URL: &str = "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/embeddinggemma-2-Q8_0.gguf";
const VISION_URL: &str = "https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/resolve/main/mmproj-embeddinggemma-2-Q8_0.gguf";

/// One EmbeddingGemma 2 process. A command starts it on the first embed and
/// stops it on exit, unless `PHOTO_ENGINE_GEMMA_URL` already points at one.
pub struct GemmaEmbedder {
    inner: Mutex<Server>,
}

struct Server {
    child: Option<Child>,
}

impl GemmaEmbedder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Server { child: None }),
        }
    }

    /// Load the model and print `ready http://127.0.0.1:<port>` when it answers.
    pub fn serve(&self, port: u16) -> Result<()> {
        let url = self.ensure(if port == 0 { None } else { Some(port) })?;
        println!("ready {url}");
        let _ = std::io::stdout().flush();
        let child = self.lock().child.take();
        if let Some(mut child) = child {
            let status = child.wait()?;
            if !status.success() {
                anyhow::bail!("llama-server exited with {status}");
            }
        }
        Ok(())
    }

    fn ensure(&self, port: Option<u16>) -> Result<String> {
        if let Ok(url) = std::env::var("PHOTO_ENGINE_GEMMA_URL") {
            let url = url.trim().trim_end_matches('/').to_string();
            if health(&url) {
                return Ok(url);
            }
            anyhow::bail!(
                "{url} is not answering. Start `photo-engine serve`, or unset PHOTO_ENGINE_GEMMA_URL."
            );
        }
        let mut guard = self.lock();
        if let Some(url) = remembered_url() {
            if health(&url) {
                return Ok(url);
            }
        }
        let (child, url) = start_server(port)?;
        guard.child = Some(child);
        Ok(url)
    }
}

fn start_server(port: Option<u16>) -> Result<(Child, String)> {
    let bin = ensure_server()?;
        let (text, vision) = ensure_weights()?;
        let port = match port {
            Some(port) => port,
            None => free_port()?,
        };
        let log_path = runtime_dir().join("llama-server.log");
        let log = File::create(&log_path)
            .with_context(|| format!("Failed to write {}", log_path.display()))?;
        let stderr = log.try_clone()?;
        eprintln!("loading EmbeddingGemma 2 on 127.0.0.1:{port}");
        let mut child = Command::new(&bin)
            .current_dir(bin.parent().unwrap_or(Path::new(".")))
            .arg("-m")
            .arg(&text)
            .arg("--mmproj")
            .arg(&vision)
            .arg("--embedding")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("-c")
            .arg("4096")
            .arg("-b")
            .arg("2048")
            .arg("-ub")
            .arg("2048")
            .arg("-np")
            .arg("1")
            .arg("--pooling")
            .arg("mean")
            .arg("--embd-normalize")
            .arg("2")
            .arg("--flash-attn")
            .arg("off")
            .arg("--no-webui")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("Failed to start {}", bin.display()))?;
        fs::write(pid_path(), child.id().to_string())?;
        let url = format!("http://127.0.0.1:{port}");
        wait_ready(&url, &mut child, &log_path)?;
    fs::write(url_path(), &url)?;
    Ok((child, url))
}

impl GemmaEmbedder {
    fn embed_body(&self, body: Value) -> Result<[f32; EMBED_DIM]> {
        let url = self.ensure(None)?;
        let raw = post_json(&format!("{url}/v1/embeddings"), &body)?;
        let value: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Embedding response was not JSON: {}", clip(&raw)))?;
        let full = embedding_values(&value)?;
        mrl_256(&full)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Server> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }
}

impl Drop for GemmaEmbedder {
    fn drop(&mut self) {
        if let Some(mut child) = self.lock().child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl MultimodalEmbedder for GemmaEmbedder {
    fn model_version(&self) -> &str {
        MODEL_ID
    }

    fn embed_image(&self, image_path: &Path) -> Result<Option<[f32; EMBED_DIM]>> {
        let Some((bytes, mime)) = image_for_embedding(image_path)? else {
            return Ok(None);
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let body = json!({
            "model": "embeddinggemma-2",
            "input": [{
                "content": [{
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime};base64,{encoded}") }
                }]
            }]
        });
        Ok(Some(self.embed_body(body)?))
    }

    fn embed_text(&self, text: &str) -> Result<[f32; EMBED_DIM]> {
        let text = text.trim();
        let text = if text.is_empty() { "photo" } else { text };
        let body = json!({
            "input": format!("task: search result | query: {text}")
        });
        self.embed_body(body)
    }
}

/// Put the GGUF weights on `drive_root` so the disk carries the model.
pub fn publish_models(drive_root: &Path) -> Result<()> {
    let (text, vision) = ensure_weights()?;
    let dest = drive_root.join(".photo_index/models");
    fs::create_dir_all(&dest)?;
    place_file(&text, &dest.join(TEXT_FILE))?;
    place_file(&vision, &dest.join(VISION_FILE))?;
    Ok(())
}

fn model_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PHOTO_ENGINE_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    sources::state_dir().join("models")
}

fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PHOTO_ENGINE_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    sources::state_dir().join("runtime")
}

fn pid_path() -> PathBuf {
    runtime_dir().join("llama-server.pid")
}

fn url_path() -> PathBuf {
    runtime_dir().join("llama-server.url")
}

fn remembered_url() -> Option<String> {
    let text = fs::read_to_string(url_path()).ok()?;
    let url = text.trim().to_string();
    if url.starts_with("http") {
        Some(url)
    } else {
        None
    }
}

fn ensure_server() -> Result<PathBuf> {
    let root = runtime_dir().join(LLAMA_TAG);
    if let Some(bin) = find_named(&root, "llama-server", 3) {
        return Ok(bin);
    }
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        anyhow::bail!(
            "The pinned llama.cpp build is for macOS Apple Silicon. This machine is {} {}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    fs::create_dir_all(&root)?;
    let archive = root.join("llama-server.tar.gz");
    download_file(LLAMA_URL, &archive, LLAMA_TAR_BYTES)?;
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&root)
        .status()
        .context("Failed to run tar")?;
    if !status.success() {
        anyhow::bail!("tar could not extract {}", archive.display());
    }
    find_named(&root, "llama-server", 3)
        .ok_or_else(|| anyhow!("llama-server was not in {}", root.display()))
}

fn ensure_weights() -> Result<(PathBuf, PathBuf)> {
    for dir in weight_dirs() {
        if let Some(pair) = weights_ready(&dir) {
            return Ok(pair);
        }
    }
    let dir = model_dir();
    fs::create_dir_all(&dir)?;
    download_file(TEXT_URL, &dir.join(TEXT_FILE), TEXT_BYTES)?;
    download_file(VISION_URL, &dir.join(VISION_FILE), VISION_BYTES)?;
    weights_ready(&dir).ok_or_else(|| anyhow!("EmbeddingGemma weights did not finish downloading"))
}

fn weight_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![model_dir()];
    if let Ok(saved) = sources::load_sources() {
        for source in saved {
            dirs.push(source.join(".photo_index/models"));
        }
    }
    let volumes = volumes_root();
    if let Ok(entries) = fs::read_dir(&volumes) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join(".photo_index/models"));
        }
    }
    dirs
}

fn weights_ready(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let text = dir.join(TEXT_FILE);
    let vision = dir.join(VISION_FILE);
    let text_ok = fs::metadata(&text).map(|meta| meta.len() == TEXT_BYTES).unwrap_or(false);
    let vision_ok = fs::metadata(&vision)
        .map(|meta| meta.len() == VISION_BYTES)
        .unwrap_or(false);
    if text_ok && vision_ok {
        Some((text, vision))
    } else {
        None
    }
}

fn place_file(src: &Path, dest: &Path) -> Result<()> {
    if let (Ok(left), Ok(right)) = (fs::canonicalize(src), fs::canonicalize(dest)) {
        if left == right {
            return Ok(());
        }
    }
    if dest.is_file() && fs::metadata(src)?.len() == fs::metadata(dest)?.len() {
        return Ok(());
    }
    if dest.exists() {
        fs::remove_file(dest)?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::hard_link(src, dest).is_ok() {
        return Ok(());
    }
    eprintln!(
        "copying {} to {}",
        src.file_name().unwrap_or_default().to_string_lossy(),
        dest.display()
    );
    fs::copy(src, dest)?;
    Ok(())
}

fn download_file(url: &str, dest: &Path, expected: u64) -> Result<()> {
    if dest.is_file() && fs::metadata(dest)?.len() == expected {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    eprintln!(
        "downloading {name} ({:.0} MB)",
        expected as f64 / 1_000_000.0
    );
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(180))
        .user_agent("photo-engine")
        .build();
    let response = agent
        .get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let partial = dest.with_extension("partial");
    let mut reader = response.into_reader();
    let mut file = File::create(&partial)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    let mut next_report = 32 * 1024 * 1024;
    loop {
        let read = reader.read(&mut buf)?;
        if read == 0 {
            break;
        }
        file.write_all(&buf[..read])?;
        got += read as u64;
        if got >= next_report {
            eprintln!(
                "  {name}  {:.0} / {:.0} MB",
                got as f64 / 1_000_000.0,
                expected as f64 / 1_000_000.0
            );
            next_report += 32 * 1024 * 1024;
        }
    }
    file.sync_all()?;
    drop(file);
    if got != expected {
        let _ = fs::remove_file(&partial);
        anyhow::bail!("download of {name} stopped at {got} bytes, expected {expected}");
    }
    fs::rename(&partial, dest)?;
    eprintln!("saved {}", dest.display());
    Ok(())
}

fn find_named(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !dir.is_dir() {
        return None;
    }
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|file| file.to_str()) == Some(name) && path.is_file() {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_named(&path, name, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn health(url: &str) -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    matches!(agent.get(&format!("{url}/health")).call(), Ok(response) if response.status() == 200)
}

fn wait_ready(url: &str, child: &mut Child, log_path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "llama-server exited with {status}\n{}",
                log_tail(log_path)
            );
        }
        if health(url) {
            return Ok(());
        }
        if Instant::now() > deadline {
            anyhow::bail!(
                "EmbeddingGemma did not become ready at {url}\n{}",
                log_tail(log_path)
            );
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn post_json(url: &str, body: &Value) -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(180))
        .build();
    let response = agent
        .post(url)
        .set("content-type", "application/json")
        .send_json(body);
    match response {
        Ok(response) => Ok(response.into_string()?),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            anyhow::bail!("embedding request failed: {code} {}", clip(&text));
        }
        Err(err) => Err(err).context("embedding request failed"),
    }
}

fn embedding_values(value: &Value) -> Result<Vec<f32>> {
    let embedding = value
        .pointer("/data/0/embedding")
        .or_else(|| value.get("embedding"))
        .ok_or_else(|| anyhow!("response has no embedding: {}", clip(&value.to_string())))?;
    flatten_embedding(embedding)
}

fn flatten_embedding(value: &Value) -> Result<Vec<f32>> {
    let Value::Array(items) = value else {
        anyhow::bail!("embedding is not an array");
    };
    if items.first().and_then(Value::as_array).is_some() {
        let mut acc = Vec::new();
        let mut count = 0f32;
        for token in items {
            let row = as_f32s(token)?;
            if acc.is_empty() {
                acc = row;
            } else if acc.len() != row.len() {
                anyhow::bail!("token rows differ in length");
            } else {
                for (slot, value) in acc.iter_mut().zip(row) {
                    *slot += value;
                }
            }
            count += 1.0;
        }
        if count == 0.0 {
            anyhow::bail!("embedding is empty");
        }
        for value in &mut acc {
            *value /= count;
        }
        return Ok(acc);
    }
    as_f32s(value)
}

fn as_f32s(value: &Value) -> Result<Vec<f32>> {
    let Value::Array(items) = value else {
        anyhow::bail!("embedding is not an array");
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let number = item
            .as_f64()
            .ok_or_else(|| anyhow!("embedding value is not a number"))?;
        out.push(number as f32);
    }
    Ok(out)
}

fn mrl_256(full: &[f32]) -> Result<[f32; EMBED_DIM]> {
    if full.len() < EMBED_DIM {
        anyhow::bail!(
            "EmbeddingGemma returned {} values, need at least {EMBED_DIM}",
            full.len()
        );
    }
    let mut out = [0f32; EMBED_DIM];
    out.copy_from_slice(&full[..EMBED_DIM]);
    if out.iter().any(|value| !value.is_finite()) {
        anyhow::bail!(
            "EmbeddingGemma returned a non-finite value. It has to run in bf16 or fp32, not fp16."
        );
    }
    let norm = out.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm < 1e-6 {
        anyhow::bail!("EmbeddingGemma returned an empty vector");
    }
    for value in &mut out {
        *value /= norm;
    }
    Ok(out)
}

fn log_tail(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    let start = text.len().saturating_sub(2000);
    text[start..].to_string()
}

fn clip(text: &str) -> String {
    let mut short: String = text.chars().take(500).collect();
    if text.chars().count() > 500 {
        short.push('…');
    }
    short
}
