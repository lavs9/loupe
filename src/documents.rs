//! Audio and video next to the photos.
//!
//! EmbeddingGemma 2 hears a waveform and sees pixels. An mp3 is the sound.
//! A clip is one representative frame plus the soundtrack, as a single vector.
//! PDF and office files wait for a documents release.

use anyhow::Result;
use id3::TagLike;
use image::ImageReader;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::EMBED_DIM;
use crate::embedder::MultimodalEmbedder;

#[allow(dead_code)]
const FTS_CHARS: usize = 8_000;
#[allow(dead_code)]
const DOC_CHARS: usize = 4_000;

pub struct PreparedFile {
    pub embedding: [f32; EMBED_DIM],
    pub preview: Option<image::RgbImage>,
    pub fts: String,
}

pub fn mime_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "tif" | "tiff" => Some("image/tiff"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "heic" | "heif" => Some("image/heic"),
        "avif" => Some("image/avif"),
        "nef" => Some("image/x-nikon-nef"),
        "arw" => Some("image/x-sony-arw"),
        "cr2" | "cr3" => Some("image/x-canon-cr3"),
        "dng" => Some("image/x-adobe-dng"),
        "raf" => Some("image/x-fuji-raf"),
        "orf" => Some("image/x-olympus-orf"),
        "rw2" => Some("image/x-panasonic-rw2"),
        "pef" => Some("image/x-pentax-pef"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "aac" => Some("audio/aac"),
        "wav" => Some("audio/wav"),
        "aiff" | "aif" => Some("audio/aiff"),
        "flac" => Some("audio/flac"),
        "ogg" | "opus" => Some("audio/ogg"),
        "mp4" => Some("video/mp4"),
        "m4v" => Some("video/x-m4v"),
        "mov" => Some("video/quicktime"),
        "avi" => Some("video/x-msvideo"),
        "mkv" => Some("video/x-matroska"),
        "webm" => Some("video/webm"),
        "mpg" | "mpeg" => Some("video/mpeg"),
        "3gp" => Some("video/3gpp"),
        _ => None,
    }
}

pub fn is_still(mime: &str) -> bool {
    mime.starts_with("image/")
}

#[allow(dead_code)]
pub fn is_audio(mime: &str) -> bool {
    mime.starts_with("audio/")
}

#[allow(dead_code)]
pub fn is_video(mime: &str) -> bool {
    mime.starts_with("video/")
}

#[allow(dead_code)]
pub fn is_playable(mime: &str) -> bool {
    is_audio(mime) || is_video(mime)
}

pub fn audio_album_art(path: &Path) -> Option<Vec<u8>> {
    let tag = id3::Tag::read_from_path(path).ok()?;
    let art = tag.pictures().next().map(|pic| pic.data.clone());
    art
}

#[allow(dead_code)]
pub fn audio_tag_info(path: &Path) -> Option<String> {
    let tag = id3::Tag::read_from_path(path).ok()?;
    let artist = tag.artist().map(|s| s.to_string());
    let album = tag.album().map(|s| s.to_string());
    match (artist, album) {
        (Some(a), Some(al)) => Some(format!("{a} — {al}")),
        (Some(a), None) => Some(a),
        (None, Some(al)) => Some(al),
        (None, None) => None,
    }
}

pub fn prepare(
    path: &Path,
    mime: &'static str,
    embedder: &dyn MultimodalEmbedder,
) -> Result<Option<PreparedFile>> {
    let mut fts_parts = Vec::new();
    if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
        fts_parts.push(name.to_string());
    }

    if mime.starts_with("audio/") {
        if let Some(tags) = audio_tags(path) {
            fts_parts.push(tags);
        }
        let Some(embedding) = embedder.embed_audio(path)? else {
            return Ok(None);
        };
        let preview = audio_album_art(path)
            .or_else(|| quicklook_image(path))
            .and_then(|bytes| decode_rgb(&bytes));
        return Ok(Some(PreparedFile {
            embedding,
            preview,
            fts: fts_parts.join(" "),
        }));
    }

    if mime.starts_with("video/") {
        let frame = quicklook_image(path).or_else(|| video_frames(path).into_iter().next());
        let soundtrack = crate::gemma::audio_wav_16k(path);
        let Some(embedding) =
            embedder.embed_multimodal(frame.as_deref(), soundtrack.as_deref())?
        else {
            return Ok(None);
        };
        let preview = frame.as_deref().and_then(decode_rgb);
        return Ok(Some(PreparedFile {
            embedding,
            preview,
            fts: fts_parts.join(" "),
        }));
    }

    Ok(None)
}

pub fn quicklook_image(path: &Path) -> Option<Vec<u8>> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("loupe-ql-{stamp}"));
    std::fs::create_dir_all(&dir).ok()?;
    let status = Command::new("/usr/bin/qlmanage")
        .args(["-t", "-s", "768", "-o"])
        .arg(&dir)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let bytes = match status {
        Ok(status) if status.success() => read_first_image(&dir),
        _ => None,
    };
    let _ = std::fs::remove_dir_all(&dir);
    bytes.filter(|body| body.len() > 64)
}

#[allow(dead_code)]
fn pdf_text(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 80 * 1024 * 1024 {
        return None;
    }
    let extracted = std::panic::catch_unwind(|| pdf_extract::extract_text(path).ok()).ok()??;
    let text = extracted.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[allow(dead_code)]
fn read_plain(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
        return None;
    }
    let nuls = bytes.iter().filter(|byte| **byte == 0).count();
    if nuls * 10 > bytes.len() {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn audio_tags(path: &Path) -> Option<String> {
    let tag = id3::Tag::read_from_path(path).ok()?;
    let mut parts = Vec::new();
    if let Some(title) = tag.title() {
        parts.push(title.to_string());
    }
    if let Some(artist) = tag.artist() {
        parts.push(artist.to_string());
    }
    if let Some(album) = tag.album() {
        parts.push(album.to_string());
    }
    if let Some(genre) = tag.genre() {
        parts.push(genre.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

pub fn video_thumbnail(path: &Path) -> Option<Vec<u8>> {
    video_frames(path).into_iter().next()
}

fn video_frames(path: &Path) -> Vec<Vec<u8>> {
    let Some(ffmpeg) = ffmpeg_bin() else {
        return Vec::new();
    };
    let duration = crate::player::query_file_duration(path).unwrap_or(1.0);
    let at = (duration * 0.4).clamp(0.1, (duration - 0.1).max(0.1));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!("loupe-frame-{stamp}.jpg"));
    let ok = Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{at:.2}"),
            "-i",
        ])
        .arg(path)
        .args(["-frames:v", "1", "-q:v", "4", "-y"])
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .is_some_and(|status| status.success());
    let bytes = if ok { std::fs::read(&out).ok() } else { None };
    let _ = std::fs::remove_file(&out);
    bytes.into_iter().collect()
}

fn ffmpeg_bin() -> Option<&'static str> {
    for bin in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/usr/bin/ffmpeg"] {
        if Path::new(bin).is_file() {
            return Some(bin);
        }
    }
    None
}


fn decode_rgb(bytes: &[u8]) -> Option<image::RgbImage> {
    let decoded = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    Some(decoded.thumbnail(384, 384).to_rgb8())
}

fn read_first_image(dir: &Path) -> Option<Vec<u8>> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Some(ext) = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
        else {
            continue;
        };
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
            return std::fs::read(path).ok();
        }
    }
    None
}

#[allow(dead_code)]
fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    text.chars().take(max).collect()
}
