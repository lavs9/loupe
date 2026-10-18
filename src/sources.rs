use anyhow::{Context, Result};
use std::collections::HashSet;
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

/// Read `name`, then the previous `PHOTO_ENGINE_*` name so old shells still work.
pub fn env_os(name: &str, legacy: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).or_else(|| std::env::var_os(legacy))
}

pub fn env_var(name: &str, legacy: &str) -> Result<String, std::env::VarError> {
    std::env::var(name).or_else(|_| std::env::var(legacy))
}

/// Where this Mac remembers internal folders it has indexed.
///
/// `LOUPE_STATE_DIR` overrides the directory so a check can avoid the real
/// Application Support folder. `PHOTO_ENGINE_STATE_DIR` is still read.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = env_os("LOUPE_STATE_DIR", "PHOTO_ENGINE_STATE_DIR") {
        return PathBuf::from(dir);
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let loupe = home.join("Library/Application Support/loupe");
    let legacy = home.join("Library/Application Support/photo-engine");
    if !loupe.exists() && legacy.exists() {
        if fs::rename(&legacy, &loupe).is_ok() {
            return loupe;
        }
        return legacy;
    }
    loupe
}

pub fn sources_file() -> PathBuf {
    state_dir().join("sources.txt")
}

/// Root scanned for disks that were plugged in. `LOUPE_VOLUMES` overrides `/Volumes`.
pub fn volumes_root() -> PathBuf {
    let path = env_os("LOUPE_VOLUMES", "PHOTO_ENGINE_VOLUMES")
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
    if !path_on_live_volume(drive_root) {
        return None;
    }
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

/// True when this path is not on a `/Volumes` disk, or that volume is still mounted.
///
/// After a disk is yanked, `exists()` / `canonicalize` on `/Volumes/Name/...` can
/// block. Consult the mount table first and never stat a missing volume.
pub fn path_on_live_volume(path: &Path) -> bool {
    match volume_name(path) {
        None => true,
        Some(name) => live_volume_names().contains(&name),
    }
}

fn volume_name(path: &Path) -> Option<String> {
    let root = volumes_root();
    let rel = path
        .strip_prefix(&root)
        .ok()
        .or_else(|| path.strip_prefix("/Volumes").ok())?;
    let name = rel.components().next()?.as_os_str();
    if name.is_empty() || name == "." {
        return None;
    }
    Some(name.to_string_lossy().into_owned())
}

fn live_volume_names() -> HashSet<String> {
    if env_os("LOUPE_VOLUMES", "PHOTO_ENGINE_VOLUMES").is_some() {
        return names_in_dir(&volumes_root());
    }
    names_from_mount()
}

fn names_in_dir(root: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    let Ok(entries) = fs::read_dir(root) else {
        return names;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else {
            continue;
        };
        if text.starts_with('.') {
            continue;
        }
        names.insert(text.to_string());
    }
    names
}

fn names_from_mount() -> HashSet<String> {
    let mut names = HashSet::new();
    let Ok(output) = Command::new("/sbin/mount").output() else {
        return names_in_dir(Path::new("/Volumes"));
    };
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some(rest) = line.split(" on ").nth(1) else {
            continue;
        };
        let Some((mount, _)) = rest.rsplit_once(" (") else {
            continue;
        };
        let Some(name) = mount.strip_prefix("/Volumes/") else {
            continue;
        };
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }
    names
}

/// Find the catalog that owns `start` by walking up to the mount boundary.
pub fn find_catalog(start: &Path) -> Option<Catalog> {
    if !path_on_live_volume(start) {
        return None;
    }
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
/// external and is not a disk image. The boot volume (`Macintosh HD`),
/// Recovery, and mounted installer `.dmg` files stay out. A check sets
/// `LOUPE_VOLUMES` to a plain directory, and every folder there is a disk.
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
    let synthetic = env_os("LOUPE_VOLUMES", "PHOTO_ENGINE_VOLUMES").is_some();
    let live = live_volume_names();
    let mut disks = Vec::new();
    for path in paths {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            if !live.contains(name) {
                continue;
            }
        }
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

/// `Some` when this path is an external HDD or SSD. Internal volumes,
/// Recovery, and disk images (a mounted installer `.dmg`) are `None`.
fn external_medium(path: &Path) -> Option<DiskMedium> {
    if let Some(info) = diskutil_info(path) {
        if !info.external || info.disk_image {
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
    disk_image: bool,
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
    let mut disk_image = false;
    let mut solid_state = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("Device Location:") {
            external = value.trim() == "External";
        } else if let Some(value) = line.strip_prefix("Protocol:") {
            disk_image = value.trim().eq_ignore_ascii_case("Disk Image");
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
        disk_image,
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
        if !path_on_live_volume(source) || !source.exists() {
            notes.push(SourceNote {
                path: source.clone(),
                status: "not connected",
            });
            continue;
        }
        match find_catalog(source) {
            Some(catalog) => push(catalog, &mut catalogs, &mut seen),
            None => notes.push(SourceNote {
                path: source.clone(),
                status: "no index",
            }),
        }
    }

    for source in load_sources()? {
        if extra.iter().any(|flag| flag == &source) {
            continue;
        }
        if !path_on_live_volume(&source) || !source.exists() {
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
