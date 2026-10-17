use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A catalog that already lives on a disk. Search reads it and does not write it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub drive_root: PathBuf,
    pub db_path: PathBuf,
    /// `BackupHDD` for an external volume, `This Mac · Pictures` for an internal folder.
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct SourceNote {
    pub path: PathBuf,
    pub status: &'static str,
}

#[derive(Debug, Clone)]
pub struct SearchSet {
    pub catalogs: Vec<Catalog>,
    pub notes: Vec<SourceNote>,
}

const CATALOG_REL: &str = ".photo_index/db/catalog.db";

/// What kind of drive a mounted volume is, when macOS says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskMedium {
    Hdd,
    Ssd,
    /// A stand-in directory, or `diskutil` did not say.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedDisk {
    pub path: PathBuf,
    pub medium: DiskMedium,
}

/// Where this Mac remembers internal folders it has indexed.
///
/// `PHOTO_ENGINE_STATE_DIR` overrides the directory so a check can avoid the real
/// Application Support folder.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PHOTO_ENGINE_STATE_DIR") {
        return PathBuf::from(dir);
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/Application Support/photo-engine")
}

pub fn sources_file() -> PathBuf {
    state_dir().join("sources.txt")
}

/// Root scanned for disks that were plugged in. `PHOTO_ENGINE_VOLUMES` overrides `/Volumes`.
pub fn volumes_root() -> PathBuf {
    let path = std::env::var_os("PHOTO_ENGINE_VOLUMES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Volumes"));
    fs::canonicalize(&path).unwrap_or(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(unix)]
fn device_id(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|meta| meta.dev())
}

#[cfg(not(unix))]
fn device_id(path: &Path) -> Option<u64> {
    let _ = path;
    Some(0)
}

fn catalog_file(drive_root: &Path) -> PathBuf {
    drive_root.join(CATALOG_REL)
}

pub fn label_for(drive_root: &Path) -> String {
    let volumes = volumes_root();
    if let Ok(rel) = drive_root.strip_prefix(&volumes) {
        if let Some(name) = rel.components().next() {
            let name = name.as_os_str().to_string_lossy();
            if !name.is_empty() && name != "." {
                return name.to_string();
            }
        }
    }
    let folder = drive_root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Mac".to_string());
    format!("This Mac · {folder}")
}

fn catalog_at(drive_root: &Path) -> Option<Catalog> {
    let drive_root = fs::canonicalize(drive_root).ok()?;
    let db_path = catalog_file(&drive_root);
    if !db_path.is_file() {
        return None;
    }
    Some(Catalog {
        label: label_for(&drive_root),
        drive_root,
        db_path,
    })
}

/// Find the catalog that owns `start` by walking up to the mount boundary.
pub fn find_catalog(start: &Path) -> Option<Catalog> {
    let start = fs::canonicalize(start).ok()?;
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start
    };
    let origin = device_id(&dir)?;
    loop {
        if let Some(catalog) = catalog_at(&dir) {
            return Some(catalog);
        }
        match dir.parent() {
            Some(parent) if parent != dir && device_id(parent) == Some(origin) => {
                dir = parent.to_path_buf();
            }
            _ => return None,
        }
    }
}

pub fn load_sources() -> Result<Vec<PathBuf>> {
    let path = sources_file();
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut sources = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        sources.push(PathBuf::from(trimmed));
    }
    Ok(sources)
}

/// Remember a folder this Mac should search. External disks are also found by the volume scan,
/// including on a machine that never indexed them.
pub fn remember(root: &Path) -> Result<()> {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut sources = load_sources()?;
    if sources.iter().any(|saved| saved == &root) {
        return Ok(());
    }
    sources.push(root);
    write_sources(&sources)
}

pub fn forget(root: &Path) -> Result<bool> {
    let canonical = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let before = load_sources()?;
    let kept: Vec<PathBuf> = before
        .iter()
        .filter(|saved| *saved != &canonical && *saved != root)
        .cloned()
        .collect();
    let removed = kept.len() != before.len();
    if removed {
        write_sources(&kept)?;
    }
    Ok(removed)
}

fn write_sources(sources: &[PathBuf]) -> Result<()> {
    let path = sources_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&path)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    writeln!(file, "# Folders this Mac searches. One path per line.")?;
    for source in sources {
        writeln!(file, "{}", source.display())?;
    }
    Ok(())
}

fn default_local_dirs() -> Vec<PathBuf> {
    home_dir()
        .map(|home| home.join("Pictures"))
        .into_iter()
        .collect()
}

/// Disks plugged in right now.
///
/// On this Mac that is each directory in `/Volumes` that `diskutil` calls
/// external. The boot volume (`Macintosh HD`) and Recovery stay out. A check
/// sets `PHOTO_ENGINE_VOLUMES` to a plain directory, and every folder there is
/// a disk.
pub fn mounted_disks() -> Vec<MountedDisk> {
    let root = volumes_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }
        paths.push(path);
    }
    paths.sort();
    let synthetic = std::env::var_os("PHOTO_ENGINE_VOLUMES").is_some();
    let mut disks = Vec::new();
    for path in paths {
        if synthetic {
            disks.push(MountedDisk {
                path,
                medium: DiskMedium::Unknown,
            });
            continue;
        }
        if let Some(medium) = external_medium(&path) {
            disks.push(MountedDisk { path, medium });
        }
    }
    disks
}

/// External disks currently mounted. A disk counts when its root holds `.photo_index`.
pub fn connected_external_catalogs() -> Vec<Catalog> {
    let mut catalogs = Vec::new();
    for disk in mounted_disks() {
        if let Some(catalog) = catalog_at(&disk.path) {
            catalogs.push(catalog);
        }
    }
    catalogs.sort_by(|a, b| a.label.cmp(&b.label));
    catalogs
}

/// `Some` when this path is an external HDD or SSD. Internal volumes are `None`.
fn external_medium(path: &Path) -> Option<DiskMedium> {
    if let Some(info) = diskutil_info(path) {
        if !info.external {
            return None;
        }
        return Some(match info.solid_state {
            Some(true) => DiskMedium::Ssd,
            Some(false) => DiskMedium::Hdd,
            None => DiskMedium::Unknown,
        });
    }
    let boot = device_id(Path::new("/"))?;
    let here = device_id(path)?;
    if here != boot {
        Some(DiskMedium::Unknown)
    } else {
        None
    }
}

struct DiskutilInfo {
    external: bool,
    solid_state: Option<bool>,
}

fn diskutil_info(path: &Path) -> Option<DiskutilInfo> {
    let output = Command::new("/usr/sbin/diskutil")
        .arg("info")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut external = false;
    let mut solid_state = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Device Location:") {
            external = value.trim() == "External";
        } else if let Some(value) = line.strip_prefix("Solid State:") {
            solid_state = match value.trim() {
                "Yes" => Some(true),
                "No" => Some(false),
                _ => None,
            };
        }
    }
    Some(DiskutilInfo {
        external,
        solid_state,
    })
}

/// Local catalogs this Mac already has, plus every connected external disk that carries an index.
///
/// `extra` are sources given on this search. Each one must resolve to a catalog.
pub fn collect(extra: &[PathBuf]) -> Result<SearchSet> {
    let mut catalogs = Vec::new();
    let mut notes = Vec::new();
    let mut seen = Vec::new();

    let push = |catalog: Catalog, catalogs: &mut Vec<Catalog>, seen: &mut Vec<PathBuf>| {
        if seen.iter().any(|path| path == &catalog.db_path) {
            return;
        }
        seen.push(catalog.db_path.clone());
        catalogs.push(catalog);
    };

    for source in extra {
        match find_catalog(source) {
            Some(catalog) => push(catalog, &mut catalogs, &mut seen),
            None if !source.exists() => {
                anyhow::bail!(
                    "{} is not connected, so there is no index to search",
                    source.display()
                );
            }
            None => {
                anyhow::bail!(
                    "no photo index on {} — index that disk first, search does not build one",
                    source.display()
                );
            }
        }
    }

    for source in load_sources()? {
        if extra.iter().any(|flag| flag == &source) {
            continue;
        }
        if !source.exists() {
            notes.push(SourceNote {
                path: source,
                status: "not connected",
            });
            continue;
        }
        match find_catalog(&source) {
            Some(catalog) => push(catalog, &mut catalogs, &mut seen),
            None => notes.push(SourceNote {
                path: source,
                status: "no index",
            }),
        }
    }

    for dir in default_local_dirs() {
        if let Some(catalog) = catalog_at(&dir) {
            push(catalog, &mut catalogs, &mut seen);
        }
    }

    for catalog in connected_external_catalogs() {
        push(catalog, &mut catalogs, &mut seen);
    }

    Ok(SearchSet { catalogs, notes })
}
