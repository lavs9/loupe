//! Folder list, shortcut chord, and update check used by the window.
//!
//! Search still goes through `sources` and `lookup`. Nothing here writes a
//! catalog except `index_folder`, which is the same path as `loupe index`.

#![cfg_attr(not(feature = "gui"), allow(dead_code))]

use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::CatalogDb;
use crate::embedder::MultimodalEmbedder;
use crate::indexer::{run_incremental_index, IndexReport, ScanTick};
use crate::lookup::{search_catalogs, RankedHit};
use crate::sources::{self, DiskMedium, SourceNote};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_SHORTCUT: &str = "shift+super+space";
const DAY_SECS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskKind {
    ThisMac,
    External,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexState {
    Ready {
        photos: i64,
        model: Option<String>,
    },
    NoIndex,
    NotConnected,
    Unreadable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderRow {
    pub path: PathBuf,
    pub label: String,
    pub kind: DiskKind,
    pub medium: DiskMedium,
    pub index: IndexState,
    pub remembered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub key: ChordKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChordKey {
    Space,
    Letter(char),
    Digit(char),
    Function(u8),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiSettings {
    pub shortcut: String,
    pub theme: String,
    pub check_updates: bool,
    pub update_feed: String,
    pub last_check_unix: u64,
}

pub const THEME_DARKROOM: &str = "darkroom";
pub const THEME_PAPER: &str = "paper";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateReport {
    NoFeed,
    /// This copy is the same as the feed, or newer.
    Current { current: String, listed: String },
    Available { current: String, latest: String, url: String },
    Failed { message: String },
}

#[derive(Debug)]
pub struct IndexOutcome {
    pub root: PathBuf,
    pub report: IndexReport,
    pub remember_error: Option<String>,
}

#[derive(Debug)]
pub struct SearchOutcome {
    pub hits: Vec<RankedHit>,
    pub index_line: String,
    pub notes: Vec<String>,
    pub errors: Vec<String>,
}

pub fn settings_path() -> PathBuf {
    sources::state_dir().join("gui.toml")
}

pub fn load_settings() -> GuiSettings {
    let mut settings = GuiSettings {
        shortcut: DEFAULT_SHORTCUT.to_string(),
        theme: THEME_DARKROOM.to_string(),
        check_updates: true,
        update_feed: String::new(),
        last_check_unix: 0,
    };
    let Ok(text) = fs::read_to_string(settings_path()) else {
        return settings;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "shortcut" => {
                let value = value.trim();
                if parse_chord(value).is_ok() {
                    settings.shortcut = value.to_string();
                }
            }
            "theme" => {
                let value = value.trim();
                if value == THEME_PAPER || value == THEME_DARKROOM {
                    settings.theme = value.to_string();
                }
            }
            "check_updates" => settings.check_updates = value.trim() != "false",
            "update_feed" => settings.update_feed = value.trim().to_string(),
            "last_check_unix" => {
                settings.last_check_unix = value.trim().parse().unwrap_or(0);
            }
            _ => {}
        }
    }
    settings
}

pub fn save_settings(settings: &GuiSettings) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = format!(
        "shortcut={}\ntheme={}\ncheck_updates={}\nupdate_feed={}\nlast_check_unix={}\n",
        settings.shortcut,
        settings.theme,
        if settings.check_updates { "true" } else { "false" },
        settings.update_feed,
        settings.last_check_unix,
    );
    fs::write(&path, body).with_context(|| format!("could not write {}", path.display()))
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn format_ist(unix: u64) -> String {
    let Some(utc) = DateTime::from_timestamp(unix as i64, 0) else {
        return "unknown time".to_string();
    };
    let Some(ist) = FixedOffset::east_opt(5 * 3600 + 30 * 60) else {
        return "unknown time".to_string();
    };
    utc.with_timezone(&ist)
        .format("%d %b %Y, %H:%M IST")
        .to_string()
}

pub fn update_is_due(settings: &GuiSettings) -> bool {
    settings.check_updates
        && is_http_url(settings.update_feed.trim())
        && unix_now().saturating_sub(settings.last_check_unix) >= DAY_SECS
}

/// Modifiers, then one key: `shift+super+space`, `ctrl+alt+super+f19`, `super+comma`.
pub fn parse_chord(text: &str) -> Result<Chord> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        bail!("The shortcut is empty.");
    }
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    if parts.iter().any(|part| part.is_empty()) {
        bail!("The shortcut has an empty piece.");
    }
    let Some((key_text, modifiers)) = parts.split_last() else {
        bail!("The shortcut is empty.");
    };
    if modifiers.is_empty() {
        bail!("The shortcut needs a modifier, so it does not swallow a normal key.");
    }
    let mut chord = Chord {
        shift: false,
        ctrl: false,
        alt: false,
        super_key: false,
        key: ChordKey::Space,
    };
    for modifier in modifiers {
        match *modifier {
            "shift" if !chord.shift => chord.shift = true,
            "ctrl" | "control" if !chord.ctrl => chord.ctrl = true,
            "alt" | "option" if !chord.alt => chord.alt = true,
            "super" | "cmd" | "command" if !chord.super_key => chord.super_key = true,
            "shift" | "ctrl" | "control" | "alt" | "option" | "super" | "cmd" | "command" => {
                bail!("The shortcut repeats {modifier}.");
            }
            other => bail!("Unknown modifier `{other}`. Use shift, ctrl, alt, and super."),
        }
    }
    chord.key = parse_key(key_text)?;
    Ok(chord)
}

fn parse_key(text: &str) -> Result<ChordKey> {
    if text == "space" {
        return Ok(ChordKey::Space);
    }
    if let Some(rest) = text.strip_prefix('f') {
        if let Ok(number) = rest.parse::<u8>() {
            if (1..=24).contains(&number) && rest == number.to_string() {
                return Ok(ChordKey::Function(number));
            }
        }
    }
    let mut chars = text.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        if ch.is_ascii_lowercase() {
            return Ok(ChordKey::Letter(ch));
        }
        if ch.is_ascii_digit() {
            return Ok(ChordKey::Digit(ch));
        }
        let named = match ch {
            '-' => "minus",
            '=' => "equal",
            ',' => "comma",
            '.' => "period",
            '/' => "slash",
            ';' => "semicolon",
            '\'' => "quote",
            '\\' => "backslash",
            '`' => "backquote",
            '[' => "bracketleft",
            ']' => "bracketright",
            _ => "",
        };
        if !named.is_empty() {
            return Ok(ChordKey::Other(named.into()));
        }
    }
    let canonical = match text {
        "return" => "enter",
        "esc" => "escape",
        "del" => "delete",
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "pgup" => "pageup",
        "pgdn" | "pagedown" => "pagedown",
        other => other,
    };
    if canonical
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        && !canonical.is_empty()
    {
        return Ok(ChordKey::Other(canonical.to_string()));
    }
    Ok(ChordKey::Other(text.to_string()))
}

pub fn format_chord(chord: &Chord) -> String {
    let mut parts = Vec::new();
    if chord.shift {
        parts.push("shift");
    }
    if chord.ctrl {
        parts.push("ctrl");
    }
    if chord.alt {
        parts.push("alt");
    }
    if chord.super_key {
        parts.push("super");
    }
    parts.push(match &chord.key {
        ChordKey::Space => "space",
        ChordKey::Letter(ch) => return format!("{}+{ch}", parts.join("+")),
        ChordKey::Digit(ch) => return format!("{}+{ch}", parts.join("+")),
        ChordKey::Function(n) => return format!("{}+f{n}", parts.join("+")),
        ChordKey::Other(name) => return format!("{}+{name}", parts.join("+")),
    });
    parts.join("+")
}

pub fn display_chord(chord: &Chord) -> String {
    let mut text = String::new();
    if chord.shift {
        text.push('⇧');
    }
    if chord.ctrl {
        text.push('⌃');
    }
    if chord.alt {
        text.push('⌥');
    }
    if chord.super_key {
        text.push('⌘');
    }
    match &chord.key {
        ChordKey::Space => text.push_str("Space"),
        ChordKey::Letter(ch) => text.push(ch.to_ascii_uppercase()),
        ChordKey::Digit(ch) => text.push(*ch),
        ChordKey::Function(n) => text.push_str(&format!("F{n}")),
        ChordKey::Other(name) => {
            let pretty = match name.as_str() {
                "enter" => "Return",
                "escape" => "Esc",
                "comma" => ",",
                "period" => ".",
                "minus" => "-",
                "equal" => "=",
                "slash" => "/",
                "up" => "↑",
                "down" => "↓",
                "left" => "←",
                "right" => "→",
                other => other,
            };
            if pretty.len() == 1 {
                text.push_str(&pretty.to_ascii_uppercase());
            } else if pretty.chars().next().is_some_and(|ch| ch.is_ascii_uppercase() || !ch.is_ascii())
            {
                text.push_str(pretty);
            } else {
                let mut chars = pretty.chars();
                if let Some(first) = chars.next() {
                    text.extend(first.to_uppercase());
                    text.push_str(chars.as_str());
                }
            }
        }
    }
    text
}

pub fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && url.len() < 2048
        && !url.chars().any(char::is_whitespace)
}

pub fn version_cmp(left: &str, right: &str) -> Result<Ordering> {
    let left = version_parts(left)?;
    let right = version_parts(right)?;
    let width = left.len().max(right.len());
    for index in 0..width {
        let order = left
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&right.get(index).copied().unwrap_or(0));
        if order != Ordering::Equal {
            return Ok(order);
        }
    }
    Ok(Ordering::Equal)
}

fn version_parts(text: &str) -> Result<Vec<u64>> {
    let text = text.trim().trim_start_matches('v');
    let text = text.split(['-', '+']).next().unwrap_or(text);
    if text.is_empty() || text.len() > 32 {
        bail!("`{text}` is not a version.");
    }
    let mut parts = Vec::new();
    for piece in text.split('.') {
        if piece.is_empty() || !piece.chars().all(|ch| ch.is_ascii_digit()) {
            bail!("`{text}` is not a version.");
        }
        parts.push(piece.parse::<u64>().unwrap_or(u64::MAX));
    }
    if parts.is_empty() {
        bail!("`{text}` is not a version.");
    }
    Ok(parts)
}

/// Read a feed body. The body is JSON: `version` and `url`. No network here.
pub fn interpret_feed(body: &str, current: &str) -> UpdateReport {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            return UpdateReport::Failed {
                message: "The feed is not JSON with a version and a url.".into(),
            };
        }
    };
    let Some(listed) = value.get("version").and_then(|item| item.as_str()) else {
        return UpdateReport::Failed {
            message: "The feed has no version.".into(),
        };
    };
    let Some(url) = value.get("url").and_then(|item| item.as_str()) else {
        return UpdateReport::Failed {
            message: "The feed has no url.".into(),
        };
    };
    if !is_http_url(url) {
        return UpdateReport::Failed {
            message: "The feed url must be http or https.".into(),
        };
    }
    match version_cmp(current, listed) {
        Ok(Ordering::Less) => UpdateReport::Available {
            current: current.to_string(),
            latest: listed.to_string(),
            url: url.to_string(),
        },
        Ok(_) => UpdateReport::Current {
            current: current.to_string(),
            listed: listed.to_string(),
        },
        Err(err) => UpdateReport::Failed {
            message: err.to_string(),
        },
    }
}

pub fn check_update(feed: &str, current: &str) -> UpdateReport {
    let feed = feed.trim();
    if feed.is_empty() {
        return UpdateReport::NoFeed;
    }
    if !is_http_url(feed) {
        return UpdateReport::Failed {
            message: "The feed must be an http or https URL.".into(),
        };
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("loupe")
        .build();
    match agent.get(feed).call() {
        Ok(response) => match response.into_string() {
            Ok(body) => interpret_feed(&body, current),
            Err(err) => UpdateReport::Failed {
                message: format!("The feed did not return text. {err}"),
            },
        },
        Err(err) => UpdateReport::Failed {
            message: format!("The update check failed. {err}"),
        },
    }
}

pub fn report_text(report: &UpdateReport) -> String {
    match report {
        UpdateReport::NoFeed => {
            "No update feed is set. The check stays off until a feed URL is saved.".into()
        }
        UpdateReport::Current { current, listed } if current == listed => {
            format!("This copy is {current}, the same as the feed.")
        }
        UpdateReport::Current { current, listed } => {
            format!("This copy is {current}. The feed lists {listed}, which is not newer.")
        }
        UpdateReport::Available { current, latest, url } => {
            format!("Version {latest} is available. This copy is {current}. {url}")
        }
        UpdateReport::Failed { message } => message.clone(),
    }
}

pub fn release_url(report: &UpdateReport) -> Option<&str> {
    match report {
        UpdateReport::Available { url, .. } if is_http_url(url) => Some(url),
        _ => None,
    }
}

/// Remembered folders, mounted disks, and Pictures when that folder exists.
/// A row with no index is listed so it can be indexed. Search does not index it.
pub fn folder_rows() -> Result<Vec<FolderRow>> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    let mounted = sources::mounted_disks();
    let media: HashMap<PathBuf, DiskMedium> = mounted
        .iter()
        .map(|disk| {
            let key = fs::canonicalize(&disk.path).unwrap_or_else(|_| disk.path.clone());
            (key, disk.medium)
        })
        .collect();
    for source in sources::load_sources()? {
        push_row(&mut rows, &mut seen, source, true, &media)?;
    }
    for disk in mounted {
        push_row(&mut rows, &mut seen, disk.path, false, &media)?;
    }
    if let Some(pictures) = pictures_dir() {
        if pictures.is_dir() {
            push_row(&mut rows, &mut seen, pictures, false, &media)?;
        }
    }
    rows.sort_by(|left, right| {
        row_rank(left)
            .cmp(&row_rank(right))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(rows)
}

fn row_rank(row: &FolderRow) -> u8 {
    match (&row.kind, &row.index) {
        (DiskKind::External, IndexState::NotConnected) => 2,
        (DiskKind::External, _) => 0,
        (DiskKind::ThisMac, IndexState::NotConnected) => 3,
        (DiskKind::ThisMac, _) => 1,
    }
}

fn push_row(
    rows: &mut Vec<FolderRow>,
    seen: &mut HashSet<PathBuf>,
    path: PathBuf,
    remembered: bool,
    media: &HashMap<PathBuf, DiskMedium>,
) -> Result<()> {
    let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    if !seen.insert(key.clone()) {
        return Ok(());
    }
    let medium = media.get(&key).copied().unwrap_or(DiskMedium::Unknown);
    rows.push(describe_row(path, remembered, medium)?);
    Ok(())
}

fn describe_row(path: PathBuf, remembered: bool, medium: DiskMedium) -> Result<FolderRow> {
    let kind = disk_kind(&path);
    let label = sources::label_for(&path);
    let index = if !path.exists() {
        IndexState::NotConnected
    } else if let Some(catalog) = sources::find_catalog(&path) {
        match CatalogDb::open_readonly(&catalog.db_path).and_then(|db| db.stats()) {
            Ok(stats) => IndexState::Ready {
                photos: stats.assets,
                model: stats.model,
            },
            Err(err) => IndexState::Unreadable(err.to_string()),
        }
    } else {
        IndexState::NoIndex
    };
    Ok(FolderRow {
        path,
        label,
        kind,
        medium,
        index,
        remembered,
    })
}

fn disk_kind(path: &Path) -> DiskKind {
    if path.starts_with(sources::volumes_root()) {
        DiskKind::External
    } else {
        DiskKind::ThisMac
    }
}

fn pictures_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Pictures"))
}

pub fn index_state_name(state: &IndexState) -> &'static str {
    match state {
        IndexState::Ready { .. } => "ready",
        IndexState::NoIndex => "no-index",
        IndexState::NotConnected => "not-connected",
        IndexState::Unreadable(_) => "unreadable",
    }
}

/// Counts the window shows while an index is running.
#[derive(Debug, Clone, Default)]
pub struct IndexLive {
    pub file: String,
    pub embedded: usize,
    pub unchanged: usize,
    pub moved: usize,
}

pub fn index_folder(root: &Path, embedder: &dyn MultimodalEmbedder) -> Result<IndexOutcome> {
    let live = Arc::new(Mutex::new(IndexLive::default()));
    index_folder_live(root, embedder, &live)
}

pub fn index_folder_live(
    root: &Path,
    embedder: &dyn MultimodalEmbedder,
    live: &Arc<Mutex<IndexLive>>,
) -> Result<IndexOutcome> {
    let root = fs::canonicalize(root).with_context(|| format!("{} is not available", root.display()))?;
    crate::gemma::publish_models(&root)?;
    let db_path = root.join(".photo_index/db/catalog.db");
    let mut db = CatalogDb::open(&db_path)?;
    let report = run_incremental_index(&root, &mut db, embedder, &mut |tick: ScanTick| {
        if let Ok(mut slot) = live.lock() {
            slot.file = tick.file;
            slot.embedded = tick.embedded;
            slot.unchanged = tick.unchanged;
            slot.moved = tick.moved;
        }
    })?;
    let remember_error = sources::remember(&root).err().map(|err| err.to_string());
    Ok(IndexOutcome {
        root,
        report,
        remember_error,
    })
}

pub fn search_photos(embedder: &dyn MultimodalEmbedder, query: &str, limit: usize) -> Result<SearchOutcome> {
    let set = sources::collect(&[])?;
    if set.catalogs.is_empty() {
        return Ok(SearchOutcome {
            hits: Vec::new(),
            index_line: "No indexes to search.".into(),
            notes: set.notes.iter().map(note_sentence).collect(),
            errors: Vec::new(),
        });
    }
    let (_parsed, hits, errors) = search_catalogs(&set, embedder, query, limit)?;
    let names = set
        .catalogs
        .iter()
        .map(|catalog| catalog.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let index_line = format!(
        "{} {}: {names}",
        set.catalogs.len(),
        if set.catalogs.len() == 1 { "index" } else { "indexes" }
    );
    Ok(SearchOutcome {
        hits,
        index_line,
        notes: set.notes.iter().map(note_sentence).collect(),
        errors,
    })
}

fn note_sentence(note: &SourceNote) -> String {
    match note.status {
        "not connected" => format!("{} is not connected.", note.path.display()),
        "no index" => format!("{} has no index yet.", note.path.display()),
        other => format!("{other} {}", note.path.display()),
    }
}

/// A sharp thumbnail for the window, cached beside the app settings.
///
/// The catalog preview used to be 32×32. Scaling that up is what made the
/// contact sheet blurry, so the grid reads the original photo instead.
pub fn display_thumb(photo: &Path) -> Result<PathBuf> {
    let meta = fs::metadata(photo).with_context(|| format!("could not read {}", photo.display()))?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_secs())
        .unwrap_or(0);
    let key = blake3::hash(format!("{}:{}:{stamp}", photo.display(), meta.len()).as_bytes());
    let destination = sources::state_dir()
        .join("gui-thumbs")
        .join(format!("{}.jpg", key.to_hex()));
    if destination.is_file() {
        return Ok(destination);
    }
    write_preview_edge(photo, &destination, 512)?;
    Ok(destination)
}

pub fn write_preview(photo: &Path, destination: &Path) -> Result<()> {
    write_preview_edge(photo, destination, 960)
}

fn write_preview_edge(photo: &Path, destination: &Path, edge: u32) -> Result<()> {
    let Some((bytes, _)) = crate::embedder::image_for_embedding(photo)? else {
        bail!("this file has no image to preview");
    };
    let image = image::load_from_memory(&bytes)
        .with_context(|| format!("could not decode {}", photo.display()))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    image
        .thumbnail(edge, edge)
        .save(destination)
        .with_context(|| format!("could not write {}", destination.display()))?;
    Ok(())
}

pub fn open_path(path: &Path) -> Result<()> {
    reveal_or_open(path, false)
}

pub fn reveal_path(path: &Path) -> Result<()> {
    reveal_or_open(path, true)
}

fn reveal_or_open(path: &Path, reveal: bool) -> Result<()> {
    if !path.exists() {
        bail!("{} is not on this Mac right now", path.display());
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        if reveal {
            command.arg("-R");
        }
        let status = command.arg(path).status()?;
        if !status.success() {
            bail!("Finder did not open {}", path.display());
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = reveal;
        bail!("Opening files from the window is implemented for macOS.");
    }
}

pub fn copy_file(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("{} is not a file on this Mac", path.display());
    }
    #[cfg(target_os = "macos")]
    {
        let path = fs::canonicalize(path)?;
        let path_str = path.to_string_lossy();
        let script = r#"function run(argv) {
    ObjC.import("AppKit");
    var pb = $.NSPasteboard.generalPasteboard;
    pb.clearContents;
    var filePath = argv[0];
    var url = $.NSURL.fileURLWithPath(filePath);
    var img = $.NSImage.alloc.initWithContentsOfFile(filePath);
    var arr = $.NSMutableArray.alloc.init;
    if (img && img.isValid) {
        arr.addObject(img);
    }
    if (url) {
        arr.addObject(url);
    }
    pb.writeObjects(arr);
    pb.setStringForType(filePath, $.NSPasteboardTypeString);
    return true;
}"#;
        let status = Command::new("osascript")
            .arg("-l")
            .arg("JavaScript")
            .arg("-e")
            .arg(script)
            .arg(path_str.as_ref())
            .status()?;
        if !status.success() {
            bail!("The clipboard did not take {}", path.display());
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("Copying the file from the window is implemented for macOS.");
    }
}

pub fn open_url(url: &str) -> Result<()> {
    if !is_http_url(url) {
        bail!("The release page is not an http URL.");
    }
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open").arg(url).status()?;
        if !status.success() {
            bail!("Could not open the release page.");
        }
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        bail!("Opening the release page is implemented for macOS.");
    }
}

pub fn print_self_check() -> Result<()> {
    let chord = parse_chord(DEFAULT_SHORTCUT)?;
    if format_chord(&chord) != DEFAULT_SHORTCUT {
        bail!("the default shortcut did not round-trip");
    }
    println!("images v1");
    println!("shortcut {}", format_chord(&chord));
    println!("display {}", display_chord(&chord));
    match parse_chord("space") {
        Ok(_) => println!("reject-bare accepted"),
        Err(_) => println!("reject-bare rejected"),
    }
    println!("order {}", order_text("0.1.0", "0.2.0")?);
    println!("order {}", order_text("0.2.0", "0.2.0")?);
    println!("order {}", order_text("0.2.0", "0.1.0")?);
    println!("order {}", order_text("0.1", "0.1.1")?);

    println!("feed-empty {}", feed_tag(&UpdateReport::NoFeed));
    let newer = interpret_feed(
        r#"{"version":"99.0.0","url":"https://example.test/loupe"}"#,
        APP_VERSION,
    );
    println!("feed-newer {}", feed_tag(&newer));
    let same = interpret_feed(
        &format!(r#"{{"version":"{APP_VERSION}","url":"https://example.test/loupe"}}"#),
        APP_VERSION,
    );
    println!("feed-same {}", feed_tag(&same));
    let older = interpret_feed(
        r#"{"version":"0.0.1","url":"https://example.test/loupe"}"#,
        APP_VERSION,
    );
    println!("feed-older {}", feed_tag(&older));
    let bad = interpret_feed("not json", APP_VERSION);
    println!("feed-bad {}", feed_tag(&bad));

    for row in folder_rows()? {
        let kind = match row.kind {
            DiskKind::ThisMac => "mac",
            DiskKind::External => "disk",
        };
        println!(
            "folder\t{kind}\t{}\t{}\t{}",
            index_state_name(&row.index),
            row.label,
            row.path.display()
        );
    }
    Ok(())
}

fn order_text(left: &str, right: &str) -> Result<String> {
    let sign = match version_cmp(left, right)? {
        Ordering::Less => "<",
        Ordering::Equal => "=",
        Ordering::Greater => ">",
    };
    Ok(format!("{left} {sign} {right}"))
}

fn feed_tag(report: &UpdateReport) -> String {
    match report {
        UpdateReport::NoFeed => "no-feed".into(),
        UpdateReport::Current { .. } => "current".into(),
        UpdateReport::Available { latest, url, .. } => format!("available {latest} {url}"),
        UpdateReport::Failed { .. } => "failed".into(),
    }
}

pub fn access_blocked(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            io.kind() == std::io::ErrorKind::PermissionDenied || io.raw_os_error() == Some(1)
        })
    })
}

pub fn index_failure(root: &Path, err: &anyhow::Error) -> String {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("That folder");
    if access_blocked(err) {
        format!(
            "{name} could not be indexed. macOS blocked it. Allow the app that opened Loupe under System Settings, Privacy & Security, Files and Folders, then try Update again."
        )
    } else {
        format!("{name} could not be indexed. {err}")
    }
}

pub fn index_summary(outcome: &IndexOutcome) -> String {
    let IndexOutcome {
        root,
        report,
        remember_error,
    } = outcome;
    let mut text = format!(
        "Indexed {}. {} unchanged, {} moved, {} newly embedded, {} removed.",
        root.display(),
        report.unchanged,
        report.moved,
        report.embedded,
        report.removed
    );
    if report.skipped == 1 {
        text.push_str(" 1 photo was skipped because macOS would not open it.");
    } else if report.skipped > 1 {
        text.push_str(&format!(
            " {} photos were skipped because macOS would not open them.",
            report.skipped
        ));
    }
    if let Some(err) = remember_error {
        text.push_str(&format!(" This Mac did not remember the folder. {err}"));
    }
    text
}
