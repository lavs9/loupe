use anyhow::Result;
use chrono::NaiveDateTime;
use exif::{In, Reader, Tag, Value};
use jwalk::WalkDir;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tracing::{info, warn};

use crate::db::{CatalogDb, ExifRecord};
use crate::embedder::{decode_image, MultimodalEmbedder, MODEL_ID};

#[derive(Debug, Clone, Default)]
pub struct IndexReport {
    pub unchanged: usize,
    pub moved: usize,
    pub embedded: usize,
    pub removed: usize,
    /// Photos left out because this Mac would not open them. The rest of the folder continues.
    pub skipped: usize,
}

/// One photo the indexer just considered. The window reads this while the scan runs.
#[derive(Debug, Clone, Default)]
pub struct ScanTick {
    pub file: String,
    pub embedded: usize,
    pub unchanged: usize,
    pub moved: usize,
}

pub fn write_config(drive_root: &Path) -> Result<()> {
    let dir = drive_root.join(".photo_index");
    std::fs::create_dir_all(dir.join("models"))?;
    std::fs::create_dir_all(dir.join("cache/previews"))?;
    std::fs::create_dir_all(dir.join("db"))?;
    let mut file = File::create(dir.join("config.toml"))?;
    writeln!(file, "model = \"{MODEL_ID}\"")?;
    writeln!(file, "dimensions = {}", crate::db::EMBED_DIM)?;
    Ok(())
}

pub fn run_incremental_index(
    drive_root: &Path,
    db: &mut CatalogDb,
    embedder: &dyn MultimodalEmbedder,
    tick: &mut dyn FnMut(ScanTick),
) -> Result<IndexReport> {
    write_config(drive_root)?;
    let photos_dir = drive_root.join("Photos");
    let scan_root = if photos_dir.is_dir() {
        photos_dir
    } else {
        drive_root.to_path_buf()
    };
    let scan_prefix = scan_root
        .strip_prefix(drive_root)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();

    info!("Scanning directory {:?} for media assets...", scan_root);
    let reembed = db.model_mismatch(embedder.model_version())?;
    if reembed {
        info!(
            "Catalog was built with a different model. Re-embedding into {}.",
            embedder.model_version()
        );
    }

    let mut report = IndexReport::default();
    let mut seen = HashSet::new();

    for entry in WalkDir::new(&scan_root)
        .skip_hidden(true)
        .process_read_dir(|_depth, _path, _state, children| {
            children.retain(|child| {
                child
                    .as_ref()
                    .ok()
                    .and_then(|entry| entry.file_name().to_str())
                    .map(|name| name != "Search" && name != ".photo_index")
                    .unwrap_or(true)
            });
        })
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(mime) = classify_media_extension(&path) else {
            continue;
        };
        let Ok(rel) = path.strip_prefix(drive_root) else {
            continue;
        };
        let rel_path = rel.to_string_lossy().to_string();

        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                warn!("skip {rel_path}: {err}");
                seen.insert(rel_path.clone());
                report.skipped += 1;
                emit(tick, &report, &rel_path);
                continue;
            }
        };
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|time| time.as_secs() as i64)
            .unwrap_or(0);
        seen.insert(rel_path.clone());

        if !reembed && db.is_path_unchanged(&rel_path, mtime, size)? {
            report.unchanged += 1;
            emit(tick, &report, &rel_path);
            continue;
        }

        let blake3_hash = match hash_file(&path) {
            Ok(hash) => hash,
            Err(err) => {
                warn!("skip {rel_path}: {err}");
                report.skipped += 1;
                emit(tick, &report, &rel_path);
                continue;
            }
        };

        if !reembed && db.has_hash(&blake3_hash)? {
            db.bind_path_to_hash(&blake3_hash, &rel_path, mtime, size)?;
            report.moved += 1;
            info!("Relinked moved photo without re-embedding: {rel_path}");
            emit(tick, &report, &rel_path);
            continue;
        }

        let exif = extract_exif(&path).unwrap_or_default();
        let decoded = decode_image(&path);
        if let Some(image) = &decoded {
            save_preview(drive_root, &blake3_hash, image);
        }
        let embedding = match embedder.embed_image(&path) {
            Ok(Some(embedding)) => embedding,
            Ok(None) => {
                warn!("skip {rel_path}: no decodable image");
                emit(tick, &report, &rel_path);
                continue;
            }
            Err(err) if access_blocked(&err) => {
                warn!("skip {rel_path}: {err}");
                report.skipped += 1;
                emit(tick, &report, &rel_path);
                continue;
            }
            Err(err) => return Err(err),
        };

        db.insert_full_asset(
            &blake3_hash,
            &rel_path,
            mtime,
            size,
            mime,
            embedder.model_version(),
            &exif,
            &embedding,
        )?;

        report.embedded += 1;
        emit(tick, &report, &rel_path);
        info!(
            "Indexed [{}]: {} (Focal: {:?}mm, f/{:?})",
            &blake3_hash[..8],
            rel_path,
            exif.focal_length_mm,
            exif.aperture_f
        );
    }

    report.removed = db.prune_missing(&scan_prefix, &seen)?;
    info!(
        "Index sync complete — unchanged: {}, moved: {}, embedded: {}, removed: {}, skipped: {}",
        report.unchanged, report.moved, report.embedded, report.removed, report.skipped
    );
    Ok(report)
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    if hasher.update_mmap_rayon(path).is_ok() {
        return Ok(hasher.finalize().to_hex().to_string());
    }
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(&mut file)?;
    Ok(hasher.finalize().to_hex().to_string())
}

fn access_blocked(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            io.kind() == std::io::ErrorKind::PermissionDenied || io.raw_os_error() == Some(1)
        })
    })
}

fn emit(tick: &mut dyn FnMut(ScanTick), report: &IndexReport, file: &str) {
    tick(ScanTick {
        file: file.to_string(),
        embedded: report.embedded,
        unchanged: report.unchanged,
        moved: report.moved,
    });
}

fn save_preview(drive_root: &Path, blake3_hash: &str, image: &image::RgbImage) {
    let path = preview_path(drive_root, blake3_hash);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = image.save(path);
}

pub(crate) fn preview_path(drive_root: &Path, blake3_hash: &str) -> PathBuf {
    let stem: String = blake3_hash.chars().take(16).collect();
    drive_root
        .join(".photo_index/cache/previews")
        .join(format!("{stem}.jpg"))
}

fn classify_media_extension(path: &Path) -> Option<&'static str> {
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
        _ => None,
    }
}

fn extract_exif(path: &Path) -> Result<ExifRecord> {
    let file = File::open(path)?;
    let mut bufreader = BufReader::new(&file);
    let exif = Reader::new().read_from_container(&mut bufreader)?;

    let mut rec = ExifRecord::default();

    if let Some(f) = exif.get_field(Tag::Make, In::PRIMARY) {
        rec.camera_make = Some(f.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(f) = exif.get_field(Tag::Model, In::PRIMARY) {
        rec.camera_model = Some(f.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(f) = exif.get_field(Tag::LensModel, In::PRIMARY) {
        rec.lens_model = Some(f.display_value().to_string().trim_matches('"').to_string());
    }
    if let Some(f) = exif.get_field(Tag::FocalLength, In::PRIMARY) {
        if let Value::Rational(ref v) = f.value {
            if let Some(r) = v.first() {
                if r.denom != 0 {
                    rec.focal_length_mm = Some(r.num as f64 / r.denom as f64);
                }
            }
        }
    }
    if let Some(f) = exif.get_field(Tag::FNumber, In::PRIMARY) {
        if let Value::Rational(ref v) = f.value {
            if let Some(r) = v.first() {
                if r.denom != 0 {
                    rec.aperture_f = Some(r.num as f64 / r.denom as f64);
                }
            }
        }
    }
    if let Some(f) = exif.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
        if let Some(iso) = f.value.get_uint(0) {
            rec.iso = Some(iso);
        }
    }
    if let Some(f) = exif.get_field(Tag::DateTimeOriginal, In::PRIMARY) {
        let raw_dt = f.display_value().to_string().trim_matches('"').to_string();
        if let Ok(ndt) = NaiveDateTime::parse_from_str(&raw_dt, "%Y-%m-%d %H:%M:%S") {
            rec.captured_at = Some(ndt.and_utc().timestamp());
        }
    }

    Ok(rec)
}
