use anyhow::Result;
use image::ImageReader;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::EMBED_DIM;
use crate::preview::largest_embedded_jpeg;

/// EmbeddingGemma 2 740M via LiteRT-LM, Matryoshka-truncated to 256.
///
/// The model emits 768 values. The catalog stores the first 256, normalized
/// again, which is the truncation EmbeddingGemma 2 was trained for.
pub const MODEL_ID: &str = "embeddinggemma-2-litert-740m-256";

pub trait MultimodalEmbedder: Send + Sync {
    fn model_version(&self) -> &str;
    /// `Ok(None)` means the file has no decodable pixels and should be skipped.
    fn embed_image(&self, image_path: &Path) -> Result<Option<[f32; EMBED_DIM]>>;
    /// `Ok(None)` means the file has no decodable waveform.
    fn embed_audio(&self, audio_path: &Path) -> Result<Option<[f32; EMBED_DIM]>>;
    /// One vector from a still and/or a soundtrack. Used for video.
    fn embed_multimodal(
        &self,
        image: Option<&[u8]>,
        audio: Option<&[u8]>,
    ) -> Result<Option<[f32; EMBED_DIM]>>;
    fn embed_text(&self, text: &str) -> Result<[f32; EMBED_DIM]>;
    fn embed_document(&self, text: &str) -> Result<[f32; EMBED_DIM]>;
}

/// Decode a photo to a preview that fits in 384×384, keeping its shape.
///
/// JPEG, PNG, TIFF, WebP and GIF go through the image crate. HEIC, HEIF and
/// AVIF are converted to JPEG by macOS ImageIO (`sips`) first, so an iPhone
/// photo is the picture as shown, including its orientation. Camera RAW is
/// read from the embedded JPEG preview so the sensor data is never demosaiced.
pub fn decode_image(path: &Path) -> Option<image::RgbImage> {
    if let Some(jpeg) = transcode_still(path) {
        if let Some(decoded) = decode_bytes(&jpeg) {
            return Some(preview_rgb(decoded));
        }
    }
    if let Some(decoded) = decode_full(path) {
        return Some(preview_rgb(decoded));
    }
    let bytes = std::fs::read(path).ok()?;
    let jpeg = largest_embedded_jpeg(&bytes)?;
    let decoded = ImageReader::new(Cursor::new(jpeg))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    Some(preview_rgb(decoded))
}

fn preview_rgb(decoded: image::DynamicImage) -> image::RgbImage {
    decoded.thumbnail(384, 384).to_rgb8()
}

/// Bytes the vision encoder should see, with a mime type that matches the bytes.
///
/// The type comes from the file header, not the extension. A `.jpg` that is
/// really a PNG is sent as PNG. HEIC and AVIF are sent as the JPEG ImageIO
/// makes from them. Camera RAW contributes an embedded JPEG or PNG preview
/// when the container itself is not an image.
pub fn image_for_embedding(path: &Path) -> Result<Option<(Vec<u8>, &'static str)>> {
    let bytes = std::fs::read(path)?;
    if let Some(image) = sniff_image(&bytes) {
        return Ok(Some((bytes, image)));
    }
    if still_container(&bytes) || still_extension(path) {
        return Ok(transcode_still(path).map(|jpeg| (jpeg, "image/jpeg")));
    }
    if let Some(jpeg) = largest_embedded_jpeg(&bytes) {
        if decode_bytes(jpeg).is_some() {
            return Ok(Some((jpeg.to_vec(), "image/jpeg")));
        }
    }
    if let Some(png) = largest_embedded_png(&bytes) {
        return Ok(Some((png, "image/png")));
    }
    Ok(decode_bytes(&bytes).and_then(|image| encode_png(&image).map(|png| (png, "image/png"))))
}

fn still_extension(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("heic" | "heif" | "avif")
    )
}

/// HEIF and AVIF start with an `ftyp` box. The brand is the four bytes after it.
fn still_container(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    matches!(
        &bytes[8..12],
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"heim" | b"heis" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" | b"heif" | b"avif" | b"avis"
    )
}

/// JPEG bytes from macOS ImageIO. `None` when this file is not a still container
/// or ImageIO cannot read it. One bad photo is skipped. The rest of the folder continues.
fn transcode_still(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut header = [0u8; 12];
    let n = std::fs::File::open(path).ok()?.read(&mut header).ok()?;
    let header = &header[..n];
    if sniff_image(header).is_some() || !(still_container(header) || still_extension(path)) {
        return None;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos())
        .unwrap_or(0);
    let out = std::env::temp_dir().join(format!("loupe-still-{stamp}.jpg"));
    let converted = Command::new("/usr/bin/sips")
        .args(["-s", "format", "jpeg", "-s", "formatOptions", "90"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .filter(|status| status.success())
        .and_then(|_| std::fs::read(&out).ok());
    let _ = std::fs::remove_file(&out);
    converted.filter(|bytes| bytes.starts_with(&[0xFF, 0xD8, 0xFF]))
}

fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    None
}

fn largest_embedded_png(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut best: Option<Vec<u8>> = None;
    let mut index = 0;
    while index + SIG.len() < bytes.len() {
        if bytes[index..].starts_with(SIG) {
            if let Some(end) = png_end(&bytes[index..]) {
                let png = bytes[index..index + end].to_vec();
                let longer = best.as_ref().map(|found| png.len() > found.len()).unwrap_or(true);
                if longer {
                    best = Some(png);
                }
                index += end.max(1);
                continue;
            }
        }
        index += 1;
    }
    best
}

fn png_end(bytes: &[u8]) -> Option<usize> {
    let mut pos = 8;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let next = pos.checked_add(12)?.checked_add(len)?;
        if next > bytes.len() {
            return None;
        }
        if kind == b"IEND" {
            return Some(next);
        }
        pos = next;
    }
    None
}

fn decode_full(path: &Path) -> Option<image::DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    decode_bytes(&bytes)
}

pub fn decode_bytes(bytes: &[u8]) -> Option<image::DynamicImage> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

pub fn encode_png(image: &image::DynamicImage) -> Option<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    image.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
    Some(cursor.into_inner())
}
