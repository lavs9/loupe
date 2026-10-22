use anyhow::{anyhow, Context, Result};
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zip::ZipArchive;

use crate::db::EMBED_DIM;
use crate::embedder::{image_for_embedding, MultimodalEmbedder, MODEL_ID};
use crate::litert::{LitertEngine, Part};
use crate::sources::{self, volumes_root};

const LITERT_TAG: &str = "v0.18.0";
const RUNTIME_ZIP: &str = "CLiteRTLM_mac.xcframework.zip";
const RUNTIME_ZIP_URL: &str = "https://github.com/google-ai-edge/LiteRT-LM/releases/download/v0.18.0/CLiteRTLM_mac.xcframework.zip";
const RUNTIME_ZIP_BYTES: u64 = 45_143_303;
const DYLIB_NAME: &str = "libCLiteRTLM_mac.dylib";
const DYLIB_BYTES: u64 = 137_802_416;
const DYLIB_ZIP_PATH: &str = "CLiteRTLM_mac.xcframework/macos-arm64_x86_64/libCLiteRTLM_mac.dylib";

const MODEL_FILE: &str = "embeddinggemma-2-740m.litertlm";
const MODEL_BYTES: u64 = 484_622_336;
const MODEL_URL: &str = "https://huggingface.co/litert-community/embeddinggemma-2-740m-litert-lm/resolve/main/embeddinggemma-2-740m.litertlm";

const LEGACY_TEXT: &str = "embeddinggemma-2-Q8_0.gguf";
const LEGACY_VISION: &str = "mmproj-embeddinggemma-2-Q8_0.gguf";

thread_local! {
    static FETCH: RefCell<Option<Arc<Mutex<FetchLive>>>> = const { RefCell::new(None) };
}

/// Bytes pulled down while the LiteRT dylib or EmbeddingGemma weights are fetched.
#[derive(Debug, Clone, Default)]
pub struct FetchLive {
    pub phase: String,
    pub got: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
}

/// True when this Mac still needs the LiteRT runtime or the 740M weights.
/// A preset `LOUPE_GEMMA_URL` skips the fetch (window e2e).
pub fn needs_fetch() -> bool {
    if sources::env_var("LOUPE_GEMMA_URL", "PHOTO_ENGINE_GEMMA_URL").is_ok() {
        return false;
    }
    !runtime_on_disk() || weights_on_disk().is_none()
}

/// Download the LiteRT dylib and the EmbeddingGemma 2 740M bundle.
pub fn prepare_runtime(live: Arc<Mutex<FetchLive>>) -> Result<()> {
    FETCH.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&live)));
    let result: Result<()> = (|| {
        let _ = ensure_runtime()?;
        let _ = ensure_weights()?;
        Ok(())
    })();
    FETCH.with(|slot| {
        if let Some(slot) = slot.borrow().as_ref() {
            if let Ok(mut guard) = slot.lock() {
                match &result {
                    Ok(()) => {
                        guard.done = true;
                        guard.phase = "ready".into();
                    }
                    Err(err) => {
                        guard.error = Some(err.to_string());
                        guard.done = true;
                    }
                }
            }
        }
        *slot.borrow_mut() = None;
    });
    result
}

pub fn weights_on_disk() -> Option<PathBuf> {
    for dir in weight_dirs() {
        if let Some(path) = model_ready(&dir) {
            return Some(path);
        }
    }
    None
}

fn runtime_on_disk() -> bool {
    dylib_ready(&runtime_dir().join(LITERT_TAG)).is_some()
}

fn report_fetch(phase: &str, got: u64, total: u64) {
    FETCH.with(|slot| {
        if let Some(live) = slot.borrow().as_ref() {
            if let Ok(mut guard) = live.lock() {
                guard.phase = phase.to_string();
                guard.got = got;
                guard.total = total;
            }
        }
    });
}

/// One EmbeddingGemma 2 engine. Loaded on the first embed, dropped with Loupe.
pub struct GemmaEmbedder {
    inner: Mutex<Option<LitertEngine>>,
}

impl GemmaEmbedder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Load the model and print `ready litert` when it answers.
    pub fn serve(&self, _port: u16) -> Result<()> {
        let _ = self.ensure()?;
        println!("ready litert");
        let _ = std::io::stdout().flush();
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    fn ensure(&self) -> Result<()> {
        let mut guard = self.lock();
        if guard.is_some() {
            return Ok(());
        }
        let dylib = ensure_runtime()?;
        let model = ensure_weights()?;
        let cache = runtime_dir().join("litert-cache");
        eprintln!("loading EmbeddingGemma 2 (LiteRT)");
        *guard = Some(LitertEngine::load(&dylib, &model, &cache)?);
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<LitertEngine>> {
        self.inner.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn embed_parts(&self, parts: &[Part<'_>]) -> Result<[f32; EMBED_DIM]> {
        self.ensure()?;
        let guard = self.lock();
        let engine = guard
            .as_ref()
            .ok_or_else(|| anyhow!("EmbeddingGemma 2 is not loaded"))?;
        engine.embed(parts)
    }
}

impl MultimodalEmbedder for GemmaEmbedder {
    fn model_version(&self) -> &str {
        MODEL_ID
    }

    fn embed_image(&self, image_path: &Path) -> Result<Option<[f32; EMBED_DIM]>> {
        let Some((bytes, _)) = image_for_embedding(image_path)? else {
            return Ok(None);
        };
        match self.embed_parts(&[Part::Image(&bytes)]) {
            Ok(embed) => Ok(Some(embed)),
            Err(err) => {
                // If LiteRT failed to decode (e.g. unknown image format, progressive JPEG, CMYK),
                // transcode via image crate to a standard PNG and retry.
                if let Some(png) = crate::embedder::decode_bytes(&bytes)
                    .and_then(|img| crate::embedder::encode_png(&img))
                {
                    if let Ok(embed) = self.embed_parts(&[Part::Image(&png)]) {
                        return Ok(Some(embed));
                    }
                }
                tracing::warn!("LiteRT could not decode {}: {err}", image_path.display());
                Ok(None)
            }
        }
    }

    fn embed_audio(&self, audio_path: &Path) -> Result<Option<[f32; EMBED_DIM]>> {
        let Some(wav) = audio_wav_16k(audio_path) else {
            return Ok(None);
        };
        Ok(Some(self.embed_parts(&[Part::Audio(&wav)])?))
    }

    fn embed_multimodal(
        &self,
        image: Option<&[u8]>,
        audio: Option<&[u8]>,
    ) -> Result<Option<[f32; EMBED_DIM]>> {
        let mut parts = Vec::new();
        if let Some(image) = image.filter(|bytes| !bytes.is_empty()) {
            parts.push(Part::Image(image));
        }
        if let Some(audio) = audio.filter(|bytes| !bytes.is_empty()) {
            parts.push(Part::Audio(audio));
        }
        if parts.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.embed_parts(&parts)?))
    }

    fn embed_text(&self, text: &str) -> Result<[f32; EMBED_DIM]> {
        let text = text.trim();
        let text = if text.is_empty() { "photo" } else { text };
        let prompt = format!("task: search query | text: {text}");
        self.embed_parts(&[Part::Text(&prompt)])
    }

    fn embed_document(&self, text: &str) -> Result<[f32; EMBED_DIM]> {
        let text = text.trim();
        let text = if text.is_empty() { "document" } else { text };
        let prompt = format!("task: search result | text: {text}");
        self.embed_parts(&[Part::Text(&prompt)])
    }
}

/// Put the LiteRT weights on `drive_root` so the disk carries the model.
pub fn publish_models(drive_root: &Path) -> Result<()> {
    let model = ensure_weights()?;
    let dest = drive_root.join(".photo_index/models");
    fs::create_dir_all(&dest)?;
    place_file(&model, &dest.join(MODEL_FILE))?;
    drop_legacy_ggufs(&dest);
    Ok(())
}

/// After a catalog is on the new model, drop llama.cpp and the old GGUFs.
pub fn retire_llama_runtime() {
    if let Ok(pid) = fs::read_to_string(runtime_dir().join("llama-server.pid")) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
    let _ = fs::remove_file(runtime_dir().join("llama-server.pid"));
    let _ = fs::remove_file(runtime_dir().join("llama-server.url"));
    let _ = fs::remove_file(runtime_dir().join("llama-server.log"));
    let _ = fs::remove_dir_all(runtime_dir().join("b11642"));
    drop_legacy_ggufs(&model_dir());
    for dir in weight_dirs() {
        drop_legacy_ggufs(&dir);
    }
}

fn drop_legacy_ggufs(dir: &Path) {
    for name in [LEGACY_TEXT, LEGACY_VISION] {
        let path = dir.join(name);
        if path.is_file() {
            let _ = fs::remove_file(&path);
        }
    }
}

fn model_dir() -> PathBuf {
    if let Some(dir) = sources::env_os("LOUPE_MODEL_DIR", "PHOTO_ENGINE_MODEL_DIR") {
        return PathBuf::from(dir);
    }
    sources::state_dir().join("models")
}

fn runtime_dir() -> PathBuf {
    if let Some(dir) = sources::env_os("LOUPE_RUNTIME_DIR", "PHOTO_ENGINE_RUNTIME_DIR") {
        return PathBuf::from(dir);
    }
    sources::state_dir().join("runtime")
}

fn ensure_runtime() -> Result<PathBuf> {
    let root = runtime_dir().join(LITERT_TAG);
    if let Some(dylib) = dylib_ready(&root) {
        adhoc_sign(&dylib);
        return Ok(dylib);
    }
    if !(cfg!(target_os = "macos") && cfg!(target_arch = "aarch64")) {
        anyhow::bail!(
            "The pinned LiteRT build is for macOS Apple Silicon. This machine is {} {}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    fs::create_dir_all(&root)?;
    let archive = root.join(RUNTIME_ZIP);
    download_file(RUNTIME_ZIP_URL, &archive, RUNTIME_ZIP_BYTES)?;
    extract_dylib(&archive, &root)?;
    let dylib = dylib_ready(&root)
        .ok_or_else(|| anyhow!("LiteRT dylib was not in {}", archive.display()))?;
    adhoc_sign(&dylib);
    Ok(dylib)
}

fn extract_dylib(archive: &Path, dest_dir: &Path) -> Result<()> {
    let file = File::open(archive)?;
    let mut zip = ZipArchive::new(file).context("LiteRT xcframework zip")?;
    let mut found = None;
    for index in 0..zip.len() {
        let name = zip.by_index(index)?.name().to_string();
        if name == DYLIB_ZIP_PATH || name.ends_with(DYLIB_NAME) {
            found = Some(index);
            break;
        }
    }
    let index = found.ok_or_else(|| anyhow!("LiteRT zip has no {DYLIB_NAME}"))?;
    let mut item = zip.by_index(index)?;
    let dest = dest_dir.join(DYLIB_NAME);
    let partial = dest.with_extension("partial");
    let mut out = File::create(&partial)?;
    std::io::copy(&mut item, &mut out)?;
    out.sync_all()?;
    drop(out);
    fs::rename(&partial, &dest)?;
    Ok(())
}

fn adhoc_sign(path: &Path) {
    let _ = Command::new("/usr/bin/codesign")
        .args(["-s", "-", "-f", "--timestamp=none"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn ensure_weights() -> Result<PathBuf> {
    for dir in weight_dirs() {
        if let Some(path) = model_ready(&dir) {
            return Ok(path);
        }
    }
    let dir = model_dir();
    fs::create_dir_all(&dir)?;
    download_file(MODEL_URL, &dir.join(MODEL_FILE), MODEL_BYTES)?;
    model_ready(&dir).ok_or_else(|| anyhow!("EmbeddingGemma weights did not finish downloading"))
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

fn model_ready(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(MODEL_FILE);
    let ok = fs::metadata(&path)
        .map(|meta| meta.len() == MODEL_BYTES)
        .unwrap_or(false);
    ok.then_some(path)
}

fn dylib_ready(dir: &Path) -> Option<PathBuf> {
    let path = dir.join(DYLIB_NAME);
    let len = fs::metadata(&path).map(|meta| meta.len()).ok()?;
    // Ad-hoc codesign changes the byte count. The zip payload is DYLIB_BYTES.
    (len == DYLIB_BYTES || (len > 100_000_000 && len < 200_000_000)).then_some(path)
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
        .timeout(Duration::from_secs(600))
        .user_agent("loupe")
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
    let mut next_report = 8 * 1024 * 1024;
    report_fetch(&name, 0, expected);
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
            report_fetch(&name, got, expected);
            next_report += 8 * 1024 * 1024;
        }
    }
    report_fetch(&name, got, expected);
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

/// Seconds of soundtrack the LiteRT 1024-token signature can take.
/// EmbeddingGemma 2 spends 25 audio tokens per second; 30 s is 750 tokens,
/// which still fits a video frame (140) in the same call.
pub const AUDIO_SECONDS: u32 = 30;
const WAV_RATE: u32 = 16_000;
const WAV_HEADER: usize = 44;

/// 16 kHz mono WAV for the audio encoder. `None` when this Mac cannot decode the file.
pub fn audio_wav_16k(path: &Path) -> Option<Vec<u8>> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!("loupe-wav-{stamp}.wav"));
    let seconds = AUDIO_SECONDS.to_string();
    let converted = if let Some(ffmpeg) = ffmpeg_bin() {
        Command::new(ffmpeg)
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-t",
                &seconds,
                "-i",
            ])
            .arg(path)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "wav", "-y"])
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .filter(|status| status.success())
            .and_then(|_| fs::read(&out).ok())
    } else {
        None
    };
    let bytes = converted.or_else(|| {
        Command::new("/usr/bin/afconvert")
            .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
            .arg(path)
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .filter(|status| status.success())
            .and_then(|_| fs::read(&out).ok())
    });
    let _ = fs::remove_file(&out);
    bytes.and_then(trim_wav_16k_mono)
}

/// Keep the first `AUDIO_SECONDS` of a 16 kHz 16-bit mono WAV. LiteRT rejects
/// anything over 1024 tokens (~41 s of audio).
fn trim_wav_16k_mono(mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.len() <= WAV_HEADER || !bytes.starts_with(b"RIFF") {
        return None;
    }
    let max_data = AUDIO_SECONDS as usize * WAV_RATE as usize * 2;
    if bytes.len() > WAV_HEADER + max_data {
        bytes.truncate(WAV_HEADER + max_data);
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        if bytes.len() >= 44 && &bytes[36..40] == b"data" {
            let data_size = max_data as u32;
            bytes[40..44].copy_from_slice(&data_size.to_le_bytes());
        }
    }
    (bytes.len() > WAV_HEADER).then_some(bytes)
}

/// Write a silent PCM WAV. Used to prove a 120 s track is cut to `AUDIO_SECONDS`.
fn write_pcm16_wav(path: &Path, rate: u32, seconds: u32) -> Result<()> {
    let samples = (rate as usize)
        .checked_mul(seconds as usize)
        .ok_or_else(|| anyhow!("wav is too long to write"))?;
    let data_bytes = samples
        .checked_mul(2)
        .ok_or_else(|| anyhow!("wav is too long to write"))?;
    let mut buf = Vec::with_capacity(WAV_HEADER + data_bytes);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&rate.to_le_bytes());
    buf.extend_from_slice(&(rate * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    buf.resize(WAV_HEADER + data_bytes, 0);
    fs::write(path, buf).with_context(|| format!("could not write {}", path.display()))
}

/// Convert a 120 s wav and fail if the encoder still sees more than 30 s.
/// LiteRT rejects anything over 1024 tokens (~41 s). Missing ffmpeg/afconvert is a fail.
pub fn print_audio_cap() -> Result<()> {
    let dir = std::env::temp_dir();
    let src = dir.join(format!(
        "loupe-audio-cap-{}.wav",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos())
            .unwrap_or(0)
    ));
    write_pcm16_wav(&src, 44_100, 120)?;
    let wav = audio_wav_16k(&src);
    let _ = fs::remove_file(&src);
    let Some(wav) = wav else {
        anyhow::bail!(
            "could not convert a 120 second wav (need ffmpeg or /usr/bin/afconvert)"
        );
    };
    let data = wav.len().saturating_sub(WAV_HEADER);
    let max = AUDIO_SECONDS as usize * WAV_RATE as usize * 2;
    if data > max {
        anyhow::bail!(
            "converted wav is {data} bytes of PCM; LiteRT only takes {max} ({} s)",
            AUDIO_SECONDS
        );
    }
    if data < WAV_RATE as usize * 2 {
        anyhow::bail!("converted wav is shorter than two seconds");
    }
    println!("audio-cap {AUDIO_SECONDS}s {data}");
    Ok(())
}

fn ffmpeg_bin() -> Option<&'static str> {
    for bin in [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
    ] {
        if Path::new(bin).is_file() {
            return Some(bin);
        }
    }
    None
}
