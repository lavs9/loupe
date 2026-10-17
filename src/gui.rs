//! Search window. The catalog, the disks, and the model stay in the rest of the crate.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, Theme, ThemeMode};
use gpui_kit::*;

use crate::app_model::{
    self, check_update, copy_file, display_chord, display_thumb, folder_rows, format_chord,
    index_failure, index_folder_live, index_summary, open_path, open_url, parse_chord, release_url,
    reveal_path,
    save_settings, search_photos, update_is_due, write_preview, Chord, ChordKey, DiskKind, FolderRow,
    GuiSettings, IndexLive, IndexState, UpdateReport, APP_VERSION,
};
use crate::gemma::GemmaEmbedder;
use crate::lookup::RankedHit;
use crate::sources::{self, DiskMedium};

actions!(
    photo,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        OpenResult,
        RevealResult,
        ToggleGrid,
        Dismiss,
        TogglePreview,
        CopyResult,
        QuitApp,
    ]
);

const SEARCH: &str = "PhotoSearch";
const RESULTS: &str = "PhotoResults";
const PRODUCT: &str = "Loupe";
const WORDMARK_FONT: &str = "Baskerville";

thread_local! {
    static HOTKEYS: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

enum Screen {
    Search,
    Folders,
}

pub fn run() -> Result<()> {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
        apply_theme(cx);
        bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1240.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(860.), px(560.))),
            window_background: WindowBackgroundAppearance::Opaque,
            titlebar: Some(TitlebarOptions {
                title: Some(PRODUCT.into()),
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(12.0), px(14.0))),
            }),
            app_owns_titlebar_drag: true,
            focus: true,
            ..Default::default()
        };
        match open_window(options, cx, |window, cx| {
            cx.new(|cx| PhotoApp::new(window, cx))
        }) {
            Ok(_) => note("window open"),
            Err(err) => {
                eprintln!("could not open the window: {err}");
                cx.quit();
            }
        }
    });
    Ok(())
}

/// The launch check reads these lines from a pipe. A pipe buffers `println!` until the process exits.
fn note(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

/// Darkroom light table. Warm black surface, paper-coloured type, safelight amber for selection.
fn apply_theme(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        let void: Hsla = rgb(0x161411).into();
        let table: Hsla = rgb(0x1C1915).into();
        let hairline: Hsla = rgb(0x3C342C).into();
        let paper: Hsla = rgb(0xEDE6DA).into();
        let mute: Hsla = rgb(0x9A9186).into();
        let safelight: Hsla = rgb(0xD4923A).into();
        let safelight_hot: Hsla = rgb(0xE0A24A).into();
        let safelight_deep: Hsla = rgb(0xC07E2C).into();
        let ink: Hsla = rgb(0x140F0A).into();
        let wash: Hsla = rgb(0x26211C).into();
        let danger: Hsla = rgb(0xC45C3E).into();
        let moss: Hsla = rgb(0x7A9A62).into();

        theme.colors.background = void;
        theme.colors.foreground = paper;
        theme.colors.title_bar = void;
        theme.colors.title_bar_border = hairline;
        theme.colors.status_bar = void;
        theme.colors.status_bar_border = hairline;
        theme.colors.border = hairline;
        theme.colors.window_border = hairline;
        theme.colors.muted = wash;
        theme.colors.muted_foreground = mute;
        theme.colors.primary = safelight;
        theme.colors.primary_foreground = ink;
        theme.colors.primary_hover = safelight_hot;
        theme.colors.primary_active = safelight_deep;
        theme.colors.button_primary = safelight;
        theme.colors.button_primary_foreground = ink;
        theme.colors.button_primary_hover = safelight_hot;
        theme.colors.button_primary_active = safelight_deep;
        theme.colors.accent = rgb(0x2A241C).into();
        theme.colors.accent_foreground = paper;
        theme.colors.secondary = wash;
        theme.colors.secondary_foreground = paper;
        theme.colors.secondary_hover = rgb(0x322C26).into();
        theme.colors.secondary_active = rgb(0x3A332C).into();
        theme.colors.button_secondary = wash;
        theme.colors.button_secondary_foreground = paper;
        theme.colors.button_secondary_hover = rgb(0x322C26).into();
        theme.colors.button_secondary_active = rgb(0x3A332C).into();
        theme.colors.ring = safelight;
        theme.colors.caret = safelight;
        theme.colors.selection = safelight.opacity(0.28);
        theme.colors.input = hairline;
        theme.colors.popover = table;
        theme.colors.popover_foreground = paper;
        theme.colors.list = void;
        theme.colors.list_hover = wash;
        theme.colors.list_active = rgb(0x2A241C).into();
        theme.colors.list_active_border = safelight;
        theme.colors.danger = danger;
        theme.colors.danger_foreground = paper;
        theme.colors.danger_hover = rgb(0xD46A4A).into();
        theme.colors.success = moss;
        theme.colors.success_foreground = paper;
        theme.colors.sidebar = void;
        theme.colors.sidebar_foreground = paper;
        theme.colors.sidebar_border = hairline;
        theme.colors.sidebar_accent = wash;
        theme.colors.sidebar_accent_foreground = paper;
        theme.colors.sidebar_primary = safelight;
        theme.colors.sidebar_primary_foreground = ink;
        theme.colors.group_box = table;
        theme.colors.group_box_foreground = paper;
        theme.colors.progress_bar = safelight;
        theme.colors.scrollbar = void;
        theme.colors.scrollbar_thumb = rgb(0x4A433A).into();
        theme.colors.scrollbar_thumb_hover = rgb(0x5A5348).into();
        theme.colors.link = safelight;
        theme.colors.link_hover = safelight_hot;
        theme.colors.tab = void;
        theme.colors.tab_active = wash;
        theme.colors.tab_active_foreground = paper;
        theme.colors.tab_foreground = mute;
        theme.colors.tab_bar = void;
        theme.radius = px(4.);
        theme.radius_lg = px(8.);
        theme.shadow = false;
    });
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", MoveUp, Some(SEARCH)),
        KeyBinding::new("down", MoveDown, Some(SEARCH)),
        KeyBinding::new("left", MoveLeft, Some(SEARCH)),
        KeyBinding::new("right", MoveRight, Some(SEARCH)),
        KeyBinding::new("enter", OpenResult, Some(SEARCH)),
        KeyBinding::new("secondary-enter", RevealResult, Some(SEARCH)),
        KeyBinding::new("secondary-o", ToggleGrid, Some(SEARCH)),
        KeyBinding::new("escape", Dismiss, Some(SEARCH)),
        KeyBinding::new("secondary-q", QuitApp, Some(SEARCH)),
        KeyBinding::new("space", TogglePreview, Some(RESULTS)),
        KeyBinding::new("secondary-c", CopyResult, Some(RESULTS)),
        KeyBinding::new("secondary-c", CopyResult, Some(SEARCH)),
    ]);
}

struct PhotoApp {
    query_input: Entity<InputState>,
    feed_input: Entity<InputState>,
    shortcut_input: Entity<InputState>,
    query: String,
    hits: Vec<RankedHit>,
    selected: usize,
    grid: bool,
    screen: Screen,
    status: SharedString,
    index_problem: Option<(PathBuf, String)>,
    index_line: SharedString,
    index_note: SharedString,
    thumbs: HashMap<PathBuf, PathBuf>,
    notes: Vec<String>,
    errors: Vec<String>,
    preview: Option<std::path::PathBuf>,
    /// Photo the side preview belongs to. Cleared with the preview so a new
    /// selection never keeps the previous picture on screen.
    preview_for: Option<PathBuf>,
    rows: Vec<FolderRow>,
    settings: GuiSettings,
    shortcut_error: String,
    update_line: String,
    release_url: Option<String>,
    busy: bool,
    generation: u64,
    embedder: Arc<GemmaEmbedder>,
    focus: FocusHandle,
    results_focus: FocusHandle,
    hits_scroll: ScrollHandle,
    search_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    thumb_task: Option<Task<()>>,
    index_task: Option<Task<()>>,
    _index_watch: Option<Task<()>>,
    update_task: Option<Task<()>>,
    _hotkey_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl PhotoApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = app_model::load_settings();
        let query_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("a wet street at night, 35mm")
        });
        let feed_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("https://example.com/loupe.json")
        });
        let shortcut_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(settings.shortcut.clone())
        });
        let saved_shortcut = settings.shortcut.clone();
        shortcut_input.update(cx, |state, cx| {
            state.replace_all(saved_shortcut, window, cx);
        });
        query_input.focus_handle(cx).focus(window, cx);
        window.on_window_should_close(cx, |_window, cx| {
            cx.hide();
            false
        });

        let subscription = cx.subscribe_in(&query_input, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                this.schedule_search(cx);
            }
        });

        let rows = folder_rows().unwrap_or_default();
        let status = if rows
            .iter()
            .any(|row| matches!(row.index, IndexState::Ready { .. }))
        {
            "Type what the photograph showed."
        } else {
            "No index yet. Open Disks and index a folder or a mounted disk."
        };
        let index_line = ready_line(&rows);
        let update_line = if settings.update_feed.is_empty() {
            "No update feed is set.".to_string()
        } else if settings.last_check_unix == 0 {
            "Not checked yet.".to_string()
        } else {
            format!("Last check {}.", app_model::format_ist(settings.last_check_unix))
        };
        let shortcut_error = match parse_chord(&settings.shortcut).and_then(|chord| install_shortcut(&chord))
        {
            Ok(()) => {
                note(&format!("shortcut registered {}", settings.shortcut));
                String::new()
            }
            Err(err) => {
                eprintln!("shortcut not registered: {err}");
                err.to_string()
            }
        };

        let focus = cx.focus_handle().tab_stop(true);
        let results_focus = cx.focus_handle().tab_stop(true);
        let due = update_is_due(&settings);
        let hotkey_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                if !hotkey_pressed() {
                    continue;
                }
                let _ = this.update_in(cx, |view, window, cx| view.present(window, cx));
            }
        });
        let mut app = Self {
            query_input,
            feed_input,
            shortcut_input,
            query: String::new(),
            hits: Vec::new(),
            selected: 0,
            grid: true,
            screen: Screen::Search,
            status: status.into(),
            index_problem: None,
            index_line: index_line.into(),
            index_note: SharedString::default(),
            thumbs: HashMap::new(),
            notes: Vec::new(),
            errors: Vec::new(),
            preview: None,
            preview_for: None,
            rows,
            settings,
            shortcut_error,
            update_line,
            release_url: None,
            busy: false,
            generation: 0,
            embedder: Arc::new(GemmaEmbedder::new()),
            focus,
            results_focus,
            hits_scroll: ScrollHandle::new(),
            search_task: None,
            preview_task: None,
            thumb_task: None,
            index_task: None,
            _index_watch: None,
            update_task: None,
            _hotkey_task: hotkey_task,
            _subscriptions: vec![subscription],
        };
        if due {
            app.spawn_update(cx);
        }
        app
    }

    fn present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Search;
        cx.activate(true);
        window.activate_window();
        self.query_input.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.preview_task = None;
        let query = self.query.trim().to_string();
        if query.is_empty() {
            self.search_task = None;
            self.hits.clear();
            self.notes.clear();
            self.errors.clear();
            self.preview = None;
            self.preview_for = None;
            self.index_line = ready_line(&self.rows).into();
            self.status = "Type what the photograph showed.".into();
            cx.notify();
            return;
        }
        self.status = "Searching…".into();
        cx.notify();
        let embedder = Arc::clone(&self.embedder);
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_spawn(async {
                std::thread::sleep(Duration::from_millis(250));
            })
            .await;
            let current = this
                .update(cx, |view, _| view.generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let outcome = cx
                .background_spawn(async move { search_photos(embedder.as_ref(), &query, 20) })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                match outcome {
                    Ok(outcome) => {
                        view.hits = outcome.hits;
                        view.selected = 0;
                        view.index_line = outcome.index_line.into();
                        view.notes = outcome.notes;
                        view.errors = outcome.errors;
                        view.status = if view.hits.is_empty() {
                            "No matching photos.".into()
                        } else {
                            format!("{} photos.", view.hits.len()).into()
                        };
                    }
                    Err(err) => {
                        view.hits.clear();
                        view.selected = 0;
                        view.status = format!("Search failed. {err}").into();
                    }
                }
                view.warm_thumbs(cx);
                if !view.hits.is_empty() {
                    view.hits_scroll.scroll_to_item(view.selected);
                }
                view.queue_side_preview(cx);
                cx.notify();
            });
        }));
    }

    fn warm_thumbs(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let jobs: Vec<PathBuf> = self
            .hits
            .iter()
            .map(|hit| hit.path.clone())
            .filter(|path| !self.thumbs.contains_key(path))
            .collect();
        if jobs.is_empty() {
            return;
        }
        self.thumb_task = Some(cx.spawn(async move |this, cx| {
            for photo in jobs {
                let current = this
                    .update(cx, |view, _| view.generation == generation)
                    .unwrap_or(false);
                if !current {
                    return;
                }
                let photo_for_job = photo.clone();
                let written = cx
                    .background_spawn(async move {
                        display_thumb(&photo_for_job)
                            .ok()
                            .map(|thumb| (photo_for_job, thumb))
                    })
                    .await;
                let Some((photo, thumb)) = written else {
                    continue;
                };
                let _ = this.update(cx, |view, cx| {
                    if view.generation != generation {
                        return;
                    }
                    let selected = view
                        .hits
                        .get(view.selected)
                        .is_some_and(|hit| hit.path == photo);
                    view.thumbs.insert(photo, thumb.clone());
                    if selected && view.preview.is_none() {
                        view.preview = Some(thumb);
                    }
                    cx.notify();
                });
            }
        }));
    }

    fn shown_thumb(&self, hit: &RankedHit) -> Option<PathBuf> {
        if let Some(path) = self.thumbs.get(&hit.path) {
            if path.is_file() {
                return Some(path.clone());
            }
        }
        let cached = &hit.preview_path;
        let sharp = cached
            .metadata()
            .map(|meta| meta.len() >= 12_000)
            .unwrap_or(false);
        if sharp && cached.is_file() {
            Some(cached.clone())
        } else {
            None
        }
    }

    fn columns(&self) -> isize {
        if !self.grid {
            return 1;
        }
        let width = f32::from(self.hits_scroll.bounds().size.width);
        if width < 160. {
            return 1;
        }
        // 216px tile, 8px gap, 12px padding on each side of the sheet (24px total padding).
        let inner = (width - 24.).max(216.);
        (((inner + 8.) / 224.).floor() as isize).max(1)
    }

    /// One row up or down, using the laid-out tiles. Before the first layout,
    /// the caller falls back to [`Self::columns`].
    fn row_step(&self, forward: bool) -> RowStep {
        let Some(current) = self.hits_scroll.bounds_for_item(self.selected) else {
            return RowStep::Unknown;
        };
        let (current_x, current_y) = item_center(current);
        let min_dy = (f32::from(current.size.height) * 0.45).max(8.);
        let mut best: Option<(usize, f32, f32)> = None;
        for index in 0..self.hits.len() {
            if index == self.selected {
                continue;
            }
            let Some(bounds) = self.hits_scroll.bounds_for_item(index) else {
                continue;
            };
            let (x, y) = item_center(bounds);
            let dy = y - current_y;
            let crossed = if forward { dy >= min_dy } else { -dy >= min_dy };
            if !crossed {
                continue;
            }
            let ady = dy.abs();
            let adx = (x - current_x).abs();
            let take = match best {
                None => true,
                Some((_, best_dy, best_dx)) => {
                    ady + 8. < best_dy || ((ady - best_dy).abs() <= min_dy && adx < best_dx)
                }
            };
            if take {
                best = Some((index, ady, adx));
            }
        }
        match best {
            Some((index, _, _)) => RowStep::Move(index),
            None => RowStep::Stay,
        }
    }

    fn move_selection(
        &mut self,
        horizontal: isize,
        vertical: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(self.screen, Screen::Search) || self.hits.is_empty() {
            return;
        }
        let last = self.hits.len() as isize - 1;
        let next = if vertical != 0 {
            match self.row_step(vertical > 0) {
                RowStep::Move(index) => index,
                RowStep::Stay => self.selected,
                RowStep::Unknown => {
                    let delta = vertical.signum() * self.columns();
                    (self.selected as isize + delta).clamp(0, last) as usize
                }
            }
        } else {
            (self.selected as isize + horizontal).clamp(0, last) as usize
        };
        if next == self.selected {
            self.hits_scroll.scroll_to_item(self.selected);
            self.results_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.select(next, window, cx);
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            return;
        }
        self.selected = index.min(self.hits.len() - 1);
        self.hits_scroll.scroll_to_item(self.selected);
        self.results_focus.focus(window, cx);
        self.queue_side_preview(cx);
        cx.notify();
    }

    fn queue_side_preview(&mut self, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(self.selected).cloned() else {
            self.preview = None;
            self.preview_for = None;
            self.preview_task = None;
            return;
        };
        let photo = hit.path.clone();
        let cached = side_cache(&photo);
        let already_sharp = self.preview_for.as_ref() == Some(&photo)
            && cached.as_ref().is_some_and(|path| {
                self.preview.as_ref() == Some(path) && file_is_large(path, 8_000)
            });
        if already_sharp {
            return;
        }
        self.preview_for = Some(photo.clone());
        self.preview = None;
        if let Some(path) = cached.as_ref() {
            if file_is_large(path, 8_000) {
                self.preview = Some(path.clone());
                return;
            }
        }
        if let Some(thumb) = self.shown_thumb(&hit) {
            self.preview = Some(thumb);
        }
        let Some(destination) = cached else {
            return;
        };
        let generation = self.generation;
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            let written = cx
                .background_spawn(async move {
                    write_preview(&photo, &destination)
                        .ok()
                        .map(|_| (photo, destination))
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some((photo, destination)) = written else {
                    return;
                };
                if view.generation != generation {
                    return;
                }
                let current = view.hits.get(view.selected).map(|hit| hit.path.clone());
                if current.as_ref() != Some(&photo) {
                    return;
                }
                view.preview_for = Some(photo);
                view.preview = Some(destination);
                cx.notify();
            });
        }));
    }

    fn open_selected(&mut self, reveal: bool, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            return;
        }
        let Some(path) = self.hits.get(self.selected).map(|hit| hit.path.clone()) else {
            self.status = "No photo selected.".into();
            cx.notify();
            return;
        };
        let result = if reveal {
            reveal_path(&path)
        } else {
            open_path(&path)
        };
        self.status = match result {
            Ok(()) if reveal => "Showing it in Finder.".into(),
            Ok(()) => format!("Opened {}", path.display()).into(),
            Err(err) => err.to_string().into(),
        };
        cx.notify();
    }

    fn copy_selected(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            return;
        }
        let Some(path) = self.hits.get(self.selected).map(|hit| hit.path.clone()) else {
            self.status = "No photo selected.".into();
            cx.notify();
            return;
        };
        self.status = match copy_file(&path) {
            Ok(()) => "Copied photo to clipboard.".into(),
            Err(err) => err.to_string().into(),
        };
        cx.notify();
    }

    fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            return;
        }
        if self.hits.get(self.selected).is_none() {
            self.status = "Move to a photo first.".into();
            cx.notify();
            return;
        }
        // The side column is the preview. Space asks for it again and does not hide it.
        self.preview_for = None;
        self.queue_side_preview(cx);
        cx.notify();
    }

    fn start_index(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        if self.busy {
            self.index_note = "An index is already running.".into();
            cx.notify();
            return;
        }
        let folder = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("folder")
            .to_string();
        self.busy = true;
        self.index_problem = None;
        self.index_note = format!("Indexing {folder}. Looking through the folder.").into();
        let live = Arc::new(Mutex::new(IndexLive::default()));
        let watch = Arc::clone(&live);
        let embedder = Arc::clone(&self.embedder);
        let folder_watch = folder.clone();
        self._index_watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let snap = watch.lock().ok().map(|slot| slot.clone());
                let keep = this
                    .update(cx, |view, cx| {
                        if !view.busy {
                            return false;
                        }
                        if let Some(snap) = snap {
                            view.index_note = index_sentence(&folder_watch, &snap).into();
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
        let failed_path = path.clone();
        self.index_task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { index_folder_live(&path, embedder.as_ref(), &live) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.busy = false;
                view.index_note = SharedString::default();
                match outcome {
                    Ok(outcome) => {
                        view.index_problem = None;
                        view.status = index_summary(&outcome).into();
                    }
                    Err(err) => {
                        let sentence = index_failure(&failed_path, &err);
                        view.status = "Could not finish indexing.".into();
                        view.index_problem = Some((failed_path, sentence));
                    }
                }
                view.reload_rows();
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn reload_rows(&mut self) {
        match folder_rows() {
            Ok(rows) => {
                self.index_line = ready_line(&rows).into();
                self.rows = rows;
            }
            Err(err) => self.status = err.to_string().into(),
        }
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Choose a folder of photos")
            .pick_folder()
        else {
            return;
        };
        if let Err(err) = sources::remember(&path) {
            self.status = err.to_string().into();
            cx.notify();
            return;
        }
        self.reload_rows();
        self.start_index(path, cx);
    }

    fn forget(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        self.status = match sources::forget(&path) {
            Ok(true) => "Removed from the remembered list. A mounted disk that still has an index stays in search.".into(),
            Ok(false) => "That folder is not in the remembered list.".into(),
            Err(err) => err.to_string().into(),
        };
        self.reload_rows();
        cx.notify();
    }

    fn save_feed(&mut self, cx: &mut Context<Self>) {
        let typed = self.feed_input.read(cx).value().to_string();
        let typed = typed.trim().to_string();
        if typed.is_empty() {
            self.update_line = "Type a feed URL, or clear the saved feed.".into();
            cx.notify();
            return;
        }
        if !app_model::is_http_url(&typed) {
            self.update_line = "The feed must be an http or https URL.".into();
            cx.notify();
            return;
        }
        self.settings.update_feed = typed;
        self.update_line = "Saved the update feed.".into();
        if let Err(err) = save_settings(&self.settings) {
            self.update_line = err.to_string();
        }
        cx.notify();
    }

    fn clear_feed(&mut self, cx: &mut Context<Self>) {
        self.settings.update_feed.clear();
        self.release_url = None;
        self.update_line = "Cleared the update feed. The check stays on this Mac.".into();
        if let Err(err) = save_settings(&self.settings) {
            self.update_line = err.to_string();
        }
        cx.notify();
    }

    fn apply_shortcut(&mut self, cx: &mut Context<Self>) {
        let typed = self.shortcut_input.read(cx).value().to_string();
        let typed = typed.trim();
        let text = if typed.is_empty() {
            self.settings.shortcut.as_str()
        } else {
            typed
        };
        match parse_chord(text) {
            Ok(chord) => match install_shortcut(&chord) {
                Ok(()) => {
                    let canonical = format_chord(&chord);
                    self.settings.shortcut = canonical;
                    self.shortcut_error.clear();
                    self.status = format!("Shortcut is {}.", display_chord(&chord)).into();
                    if let Err(err) = save_settings(&self.settings) {
                        self.shortcut_error = err.to_string();
                    }
                }
                Err(err) => self.shortcut_error = err.to_string(),
            },
            Err(err) => self.shortcut_error = err.to_string(),
        }
        cx.notify();
    }

    fn spawn_update(&mut self, cx: &mut Context<Self>) {
        let feed = self.settings.update_feed.clone();
        self.update_line = "Checking for an update…".into();
        cx.notify();
        self.update_task = Some(cx.spawn(async move |this, cx| {
            let report = cx
                .background_spawn(async move { check_update(&feed, APP_VERSION) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.apply_report(report);
                cx.notify();
            });
        }));
    }

    fn apply_report(&mut self, report: UpdateReport) {
        self.settings.last_check_unix = app_model::unix_now();
        self.release_url = release_url(&report).map(str::to_string);
        self.update_line = format!(
            "{} Last check {}.",
            app_model::report_text(&report),
            app_model::format_ist(self.settings.last_check_unix)
        );
        if let Err(err) = save_settings(&self.settings) {
            self.update_line = err.to_string();
        }
    }

    fn on_move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(0, -1, window, cx);
    }

    fn on_move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(0, 1, window, cx);
    }

    fn on_move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, 0, window, cx);
    }

    fn on_move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, 0, window, cx);
    }

    fn on_open(&mut self, _: &OpenResult, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(false, cx);
    }

    fn on_reveal(&mut self, _: &RevealResult, _: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(true, cx);
    }

    fn on_grid(&mut self, _: &ToggleGrid, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            self.screen = Screen::Search;
            self.grid = true;
        } else {
            self.grid = !self.grid;
        }
        if self.hits.is_empty() {
            self.query_input.focus_handle(cx).focus(window, cx);
        } else {
            self.hits_scroll.scroll_to_item(self.selected);
            self.results_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn on_quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.hide();
    }

    fn on_preview(&mut self, _: &TogglePreview, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_preview(cx);
    }

    fn on_copy(&mut self, _: &CopyResult, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(cx);
    }

    fn apply_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.query = text.clone();
        self.query_input.update(cx, |state, cx| {
            state.replace_all(text, window, cx);
        });
        self.schedule_search(cx);
    }

    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.clear();
        self.query_input.update(cx, |state, cx| {
            state.replace_all(String::new(), window, cx);
        });
        self.query_input.focus_handle(cx).focus(window, cx);
        self.schedule_search(cx);
    }

    fn render_top_bar(&self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let mute = cx.theme().muted_foreground;
        let paper = cx.theme().foreground;
        let safelight = cx.theme().primary;
        let is_search = matches!(self.screen, Screen::Search);
        let ready_count: i64 = self
            .rows
            .iter()
            .filter_map(|r| match &r.index {
                IndexState::Ready { photos, .. } => Some(*photos),
                _ => None,
            })
            .sum();
        let count_label = if !self.hits.is_empty() {
            format!("{}", self.hits.len())
        } else if ready_count > 0 {
            format!("{ready_count}")
        } else {
            String::new()
        };

        div()
            .w_full()
            .h(px(52.))
            .border_b_1()
            .border_color(border)
            .bg(cx.theme().background)
            .pl(px(80.))
            .pr_5()
            .flex()
            .items_center()
            .justify_between()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .flex_shrink_0()
                    .child(
                        div()
                            .font_family(WORDMARK_FONT)
                            .italic()
                            .text_lg()
                            .text_color(paper)
                            .child(PRODUCT),
                    )
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_5()
                            .child(screen_tab(
                                "tab-search",
                                "Search",
                                is_search,
                                paper,
                                mute,
                                safelight,
                                cx.listener(|this, _, window, cx| {
                                    this.present(window, cx);
                                }),
                            ))
                            .child(screen_tab(
                                "tab-folders",
                                "Disks",
                                !is_search,
                                paper,
                                mute,
                                safelight,
                                cx.listener(|this, _, _, cx| {
                                    this.screen = Screen::Folders;
                                    this.reload_rows();
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .flex_shrink_0()
                    .children(if is_search {
                        vec![
                            Button::new("toggle-grid-btn")
                                .xsmall()
                                .ghost()
                                .icon(IconName::LayoutGrid)
                                .tooltip("Contact sheet (⌘O)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if !this.grid {
                                        this.on_grid(&ToggleGrid, window, cx);
                                    }
                                }))
                                .into_any_element(),
                            Button::new("toggle-list-btn")
                                .xsmall()
                                .ghost()
                                .icon(IconName::LayoutList)
                                .tooltip("List (⌘O)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if this.grid {
                                        this.on_grid(&ToggleGrid, window, cx);
                                    }
                                }))
                                .into_any_element(),
                            div()
                                .text_xs()
                                .text_color(mute)
                                .child(count_label)
                                .into_any_element(),
                        ]
                    } else {
                        vec![
                            Button::new("top-add-folder")
                                .small()
                                .primary()
                                .icon(IconName::FolderPlus)
                                .label("Add folder")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.add_folder(cx);
                                }))
                                .into_any_element(),
                        ]
                    }),
            )
    }

    fn render_index_banner(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        div()
            .w_full()
            .px_5()
            .py_2()
            .border_b_1()
            .border_color(border)
            .bg(cx.theme().primary.opacity(0.12))
            .flex()
            .items_center()
            .gap_3()
            .child(
                Progress::new("index-progress")
                    .loading(true)
                    .w(px(96.))
                    .color(cx.theme().primary),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(self.index_note.clone()),
            )
    }

    fn welcome_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_fg = cx.theme().muted_foreground;
        let total_photos: i64 = self
            .rows
            .iter()
            .filter_map(|r| match &r.index {
                IndexState::Ready { photos, .. } => Some(*photos),
                _ => None,
            })
            .sum();

        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_8()
            .gap_5()
            .child(
                div()
                    .font_family(WORDMARK_FONT)
                    .italic()
                    .text_xl()
                    .text_center()
                    .child("What did the photograph show?"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .max_w(px(560.))
                    .children(SUGGESTED_QUERIES.iter().map(|&suggestion| {
                        Button::new(SharedString::from(format!("welcome-{suggestion}")))
                            .small()
                            .ghost()
                            .label(suggestion)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.apply_query(suggestion, window, cx);
                            }))
                    })),
            )
            .child(
                if total_photos > 0 {
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(format!("{total_photos} photographs on disk"))
                        .into_any_element()
                } else {
                    Button::new("welcome-add-folder")
                        .small()
                        .primary()
                        .icon(IconName::FolderPlus)
                        .label("Index a folder")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.add_folder(cx);
                        }))
                        .into_any_element()
                },
            )
    }

    fn no_results_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_fg = cx.theme().muted_foreground;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_8()
            .gap_3()
            .child(
                div()
                    .font_family(WORDMARK_FONT)
                    .italic()
                    .text_lg()
                    .child("Nothing matched that."),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .text_center()
                    .max_w(px(400.))
                    .child("Try fewer words, or open Disks and index the folder that holds the photos."),
            )
            .child(
                Button::new("clear-query-btn")
                    .small()
                    .ghost()
                    .label("Clear")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_query(window, cx);
                    })),
            )
    }

    fn search_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hits = self.hits.clone();
        let selected = self.selected;
        let grid = self.grid;
        let border = cx.theme().border;

        let has_hits = !hits.is_empty();
        let is_empty_query = self.query.trim().is_empty();

        let search_bar = div()
            .w_full()
            .px_5()
            .py_3()
            .border_b_1()
            .border_color(border)
            .bg(cx.theme().background)
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .child(Input::new(&self.query_input).cleanable(true)),
            )
            .children((!self.query.trim().is_empty()).then(|| {
                Button::new("clear-search-btn")
                    .small()
                    .ghost()
                    .icon(IconName::X)
                    .tooltip("Clear")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.clear_query(window, cx);
                    }))
            }));

        let list_content: AnyElement = if is_empty_query && !has_hits {
            self.welcome_view(cx).into_any_element()
        } else if !is_empty_query && !has_hits && !self.busy {
            self.no_results_view(cx).into_any_element()
        } else {
            let mut list = div()
                .id("hits")
                .track_focus(&self.results_focus)
                .track_scroll(&self.hits_scroll)
                .key_context(RESULTS)
                .on_action(cx.listener(Self::on_preview))
                .on_action(cx.listener(Self::on_copy))
                .flex_1()
                .min_w(px(0.))
                .min_h(px(0.))
                .overflow_y_scroll()
                .p_3();

            if grid {
                list = list.flex().flex_wrap().content_start().gap_2();
                for (index, hit) in hits.into_iter().enumerate() {
                    list = list.child(self.tile(index, hit, selected == index, cx));
                }
            } else {
                list = list.flex().flex_col().gap_2();
                for (index, hit) in hits.into_iter().enumerate() {
                    list = list.child(self.hit_row(index, hit, selected == index, cx));
                }
            }

            div()
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .flex()
                .overflow_hidden()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .min_h(px(0.))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .child(list),
                )
                .child(self.side_pane(cx))
                .into_any_element()
        };

        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .children(self.index_problem.as_ref().map(|(_, sentence)| {
                div()
                    .mx_6()
                    .mt_3()
                    .p_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().danger)
                    .bg(cx.theme().danger.opacity(0.1))
                    .text_xs()
                    .text_color(cx.theme().danger_foreground)
                    .child(sentence.clone())
            }))
            .children(self.notes.iter().map(|note| {
                div().mx_6().mt_1().text_xs().text_color(cx.theme().muted_foreground).child(note.clone())
            }))
            .children(self.errors.iter().map(|err| {
                div().mx_6().mt_1().text_xs().text_color(cx.theme().danger).child(err.clone())
            }))
            .child(search_bar)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .flex()
                    .child(list_content),
            )
    }

    fn side_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let mut pane = div()
            .id("side-preview")
            .w(px(320.))
            .min_h(px(0.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .border_l_1()
            .border_color(border)
            .bg(cx.theme().background)
            .p_4()
            .overflow_y_scroll();

        let Some(hit) = self.hits.get(self.selected) else {
            return pane.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .text_center()
                    .text_color(muted_fg)
                    .child(div().text_sm().child("Select a photograph")),
            );
        };

        let name = file_name(hit);
        let path = hit.path.display().to_string();
        let thumb = self.shown_thumb(hit);
        let preview_img = self.preview.as_deref().filter(|path| path.is_file())
            .or(thumb.as_deref());
        let hit_path = hit.path.clone();
        let hit_reveal = hit.path.clone();
        let hit_copy_path = hit.path.clone();

        pane = pane
            .child(
                div()
                    .w_full()
                    .h(px(280.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .bg(rgb(0x0E0C0A))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        if let Some(p) = preview_img {
                            img(Arc::<std::path::Path>::from(p.to_path_buf()))
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element()
                        } else {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(muted_fg.opacity(0.5))
                                .child(IconName::Image)
                                .into_any_element()
                        }
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(format!("{} of {}", self.selected + 1, self.hits.len())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(
                        Button::new("side-open")
                            .small()
                            .primary()
                            .icon(IconName::ExternalLink)
                            .label("Open")
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.status = match open_path(&hit_path) {
                                    Ok(()) => format!("Opened {}", hit_path.display()).into(),
                                    Err(err) => err.to_string().into(),
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("side-reveal")
                            .small()
                            .secondary()
                            .icon(IconName::Folder)
                            .label("Finder")
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.status = match reveal_path(&hit_reveal) {
                                    Ok(()) => "Showing it in Finder.".into(),
                                    Err(err) => err.to_string().into(),
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("side-copy")
                            .small()
                            .secondary()
                            .icon(IconName::Copy)
                            .label("Copy")
                            .tooltip("Copy photo to clipboard (⌘C)")
                            .flex_1()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_selected(cx);
                            })),
                    ),
            );

        let mut facts = DescriptionList::vertical()
            .columns(1)
            .bordered(false)
            .small()
            .item("On", hit.label.clone(), 1);

        if let Ok(meta) = std::fs::metadata(&hit.path) {
            let bytes = meta.len();
            let size_str = if bytes >= 1_048_576 {
                format!("{:.1} MB", bytes as f64 / 1_048_576.0)
            } else if bytes >= 1024 {
                format!("{:.0} KB", bytes as f64 / 1024.0)
            } else {
                format!("{bytes} B")
            };
            facts = facts.item("File Size", size_str, 1);
            if let Ok(modified) = meta.modified() {
                let dt: chrono::DateTime<chrono::Local> = modified.into();
                facts = facts.item("Modified", dt.format("%b %-d, %Y %-I:%M %p").to_string(), 1);
            }
        }

        if let Some(camera) = &hit.camera_model {
            facts = facts.item("Camera", camera.clone(), 1);
        }
        if let Some(mm) = hit.focal_length_mm {
            facts = facts.item("Focal Length", format!("{mm:.0} mm"), 1);
        }
        if let Some(aperture) = hit.aperture_f {
            facts = facts.item("Aperture", format!("f/{aperture:.1}"), 1);
        }

        pane.child(facts).child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_xs().text_color(muted_fg).child("Path"))
                        .child(
                            Button::new("copy-path-btn")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Copy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let path_str = hit_copy_path.display().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(path_str));
                                    this.status = "Copied file path to clipboard.".into();
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().text_xs().text_color(muted_fg).child(path)),
        )
    }

    fn hit_row(
        &mut self,
        index: usize,
        hit: RankedHit,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = file_name(&hit);
        let path = hit.path.display().to_string();
        let thumb = self.shown_thumb(&hit);
        let path_clone = hit.path.clone();
        let path_reveal = hit.path.clone();

        let bg = if selected {
            cx.theme().accent
        } else {
            cx.theme().background
        };

        let mut specs = Vec::new();
        specs.push(hit.label.clone());
        if let Some(camera) = &hit.camera_model {
            specs.push(camera.clone());
        }
        if let Some(focal) = hit.focal_length_mm {
            specs.push(format!("{focal:.0}mm"));
        }
        if let Some(aperture) = hit.aperture_f {
            specs.push(format!("f/{aperture:.1}"));
        }
        let meta_line = specs.join("  •  ");

        div()
            .id(("hit", index))
            .flex()
            .gap_3()
            .items_center()
            .px_2()
            .py_1_5()
            .border_l_2()
            .border_color(if selected {
                cx.theme().primary
            } else {
                cx.theme().primary.opacity(0.)
            })
            .bg(bg)
            .hover(|s| s.bg(cx.theme().accent))
            .child(thumb_box(thumb.as_deref(), 48., 48., cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(meta_line),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground.opacity(0.7))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(path),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1_5()
                    .child(
                        Button::new(SharedString::from(format!("open-row-{index}")))
                            .small()
                            .ghost()
                            .icon(IconName::ExternalLink)
                            .tooltip("Open photo (↵)")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = open_path(&path_clone);
                                this.status = format!("Opened {}", path_clone.display()).into();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("reveal-row-{index}")))
                            .small()
                            .ghost()
                            .icon(IconName::Folder)
                            .tooltip("Reveal in Finder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = reveal_path(&path_reveal);
                                this.status = "Showing in Finder.".into();
                                cx.notify();
                            })),
                    ),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(index, window, cx);
                if let ClickEvent::Mouse(m) = event {
                    if m.up.click_count >= 2 {
                        this.open_selected(false, cx);
                    }
                }
            }))
    }

    fn tile(
        &mut self,
        index: usize,
        hit: RankedHit,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = file_name(&hit);
        let thumb = self.shown_thumb(&hit);
        let label = hit.label.clone();
        let border_color = if selected {
            cx.theme().primary
        } else {
            cx.theme().primary.opacity(0.)
        };

        let mut camera_spec = String::new();
        if let Some(focal) = hit.focal_length_mm {
            camera_spec.push_str(&format!("{focal:.0}mm"));
        }
        if let Some(aperture) = hit.aperture_f {
            if !camera_spec.is_empty() {
                camera_spec.push_str(" • ");
            }
            camera_spec.push_str(&format!("f/{aperture:.1}"));
        }

        div()
            .id(("tile", index))
            .w(px(216.))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .w_full()
                    .h(px(156.))
                    .overflow_hidden()
                    .border_2()
                    .border_color(border_color)
                    .bg(rgb(0x0E0C0A))
                    .child(
                        if let Some(p) = thumb.as_deref().filter(|p| p.is_file()) {
                            img(Arc::<std::path::Path>::from(p.to_path_buf()))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .into_any_element()
                        } else {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_color(cx.theme().muted_foreground.opacity(0.5))
                                .child(IconName::Image)
                                .into_any_element()
                        }
                    ),
            )
            .child(
                div()
                    .px(px(2.))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .opacity(if selected { 1. } else { 0.72 })
                    .child(
                        div()
                            .text_xs()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(if selected {
                                cx.theme().foreground
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(if camera_spec.is_empty() {
                                label
                            } else {
                                format!("{label}  {camera_spec}")
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.select(index, window, cx);
                if let ClickEvent::Mouse(m) = event {
                    if m.up.click_count >= 2 {
                        this.open_selected(false, cx);
                    }
                }
            }))
    }

    fn folders_page(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows.clone();
        let mac: Vec<(usize, FolderRow)> = rows
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, row)| row.kind == DiskKind::ThisMac)
            .collect();
        let disks: Vec<(usize, FolderRow)> = rows
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, row)| row.kind == DiskKind::External)
            .collect();
        let no_disks = disks.is_empty();

        let total_photos: i64 = rows
            .iter()
            .filter_map(|r| match &r.index {
                IndexState::Ready { photos, .. } => Some(*photos),
                _ => None,
            })
            .sum();
        let ready_sources = rows
            .iter()
            .filter(|r| matches!(r.index, IndexState::Ready { .. }))
            .count();
        let summary = if ready_sources == 0 {
            "Nothing indexed yet.".to_string()
        } else {
            format!(
                "{total_photos} photographs on {ready_sources} {}",
                if ready_sources == 1 { "index" } else { "indexes" }
            )
        };

        let mut mac_list = div().id("mac-folders").flex().flex_col().gap_2();
        for (index, row) in mac {
            mac_list = mac_list.child(self.folder_row(index, row, cx));
        }

        let mut disk_list = div().id("disk-folders").flex().flex_col().gap_2();
        for (index, row) in disks {
            disk_list = disk_list.child(self.folder_row(index, row, cx));
        }

        let shortcut = shortcut_label(&self.settings);
        let problem = self.index_problem.as_ref().map(|(_, sentence)| sentence.clone());
        let muted = cx.theme().muted_foreground;

        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .child(
                div()
                    .id("folders-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.))
                    .gap_6()
                    .p_6()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .font_family(WORDMARK_FONT)
                            .italic()
                            .text_lg()
                            .child(summary),
                    )
                    .children(problem.map(|sentence| {
                        div()
                            .p_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(cx.theme().danger)
                            .bg(cx.theme().danger.opacity(0.1))
                            .text_sm()
                            .text_color(cx.theme().danger_foreground)
                            .child(sentence)
                    }))
                    // Local Folders Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_sm()
                                            .font_semibold()
                                            .child(IconName::Folder)
                                            .child("This Mac"),
                                    )
                            )
                            .child(mac_list),
                    )
                    // Disks Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .text_sm()
                                            .font_semibold()
                                            .child(IconName::HardDrive)
                                            .child("Plugged-in disks"),
                                    )
                            )
                            .children(no_disks.then(|| {
                                div()
                                    .py_3()
                                    .text_sm()
                                    .text_color(muted)
                                    .child("No disk is plugged in. A disk with .photo_index is searched as soon as it mounts.")
                            }))
                            .child(disk_list),
                    )
                    // Settings Section
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .pt_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(IconName::SlidersHorizontal)
                                    .child("Preferences"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_4()
                                    .child(
                                        // Shortcut Box
                                        GroupBox::new()
                                            .id("shortcut-box")
                                            .outline()
                                            .title("Global Shortcut")
                                            .child(format!("{shortcut} opens search from any app."))
                                            .child("Use shift, ctrl, alt, and super, then space, a letter, or f1-f24.")
                                            .child(Input::new(&self.shortcut_input))
                                            .child(div().text_xs().text_color(cx.theme().danger).child(self.shortcut_error.clone()))
                                            .child(
                                                Button::new("apply-shortcut")
                                                    .small()
                                                    .outline()
                                                    .label("Apply Shortcut")
                                                    .on_click(cx.listener(|this, _, _, cx| this.apply_shortcut(cx))),
                                            ),
                                    )
                                    .child(
                                        // Update Box
                                        GroupBox::new()
                                            .id("update-box")
                                            .outline()
                                            .title("Software Updates")
                                            .child(format!("This copy is {APP_VERSION}. The check reads one small JSON file."))
                                            .child(div().text_xs().text_color(muted).child(self.update_line.clone()))
                                            .child(Input::new(&self.feed_input))
                                            .child(
                                                Checkbox::new("daily-update")
                                                    .label("Check once a day")
                                                    .checked(self.settings.check_updates)
                                                    .on_change(cx.listener(|this, checked, _, cx| {
                                                        this.settings.check_updates = *checked;
                                                        if let Err(err) = save_settings(&this.settings) {
                                                            this.update_line = err.to_string();
                                                        }
                                                        cx.notify();
                                                    })),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .flex_wrap()
                                                    .child(
                                                        Button::new("save-feed")
                                                            .small()
                                                            .outline()
                                                            .label("Save feed")
                                                            .on_click(cx.listener(|this, _, _, cx| this.save_feed(cx))),
                                                    )
                                                    .child(
                                                        Button::new("clear-feed")
                                                            .small()
                                                            .ghost()
                                                            .label("Clear feed")
                                                            .on_click(cx.listener(|this, _, _, cx| this.clear_feed(cx))),
                                                    )
                                                    .child(
                                                        Button::new("check-now")
                                                            .small()
                                                            .secondary()
                                                            .label("Check now")
                                                            .on_click(cx.listener(|this, _, _, cx| this.spawn_update(cx))),
                                                    )
                                                    .children(self.release_url.clone().map(|url| {
                                                        Button::new("open-release")
                                                            .small()
                                                            .primary()
                                                            .label("Open Release Page")
                                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                                this.status = match open_url(&url) {
                                                                    Ok(()) => "Opened release page.".into(),
                                                                    Err(err) => err.to_string().into(),
                                                                };
                                                                cx.notify();
                                                            }))
                                                    })),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    fn folder_row(&mut self, index: usize, row: FolderRow, cx: &mut Context<Self>) -> impl IntoElement {
        let path = row.path.clone();
        let index_path = path.clone();
        let forget_path = path.clone();
        let reveal_path_val = path.clone();
        let title = folder_title(&row);
        let hint = folder_hint(&row);
        let where_line = short_path(&path);
        let indexed = matches!(row.index, IndexState::Ready { .. });
        let connected = !matches!(row.index, IndexState::NotConnected);
        let remembered = row.remembered;
        let action = if indexed { "Update" } else { "Index" };
        let muted = cx.theme().muted_foreground;
        let is_external = row.kind == DiskKind::External;
        let pip = match &row.index {
            IndexState::Ready { .. } => cx.theme().primary,
            IndexState::Unreadable(_) => cx.theme().danger,
            _ => cx.theme().muted_foreground.opacity(0.35),
        };

        let status_tag = match &row.index {
            IndexState::Ready { photos, .. } => Tag::secondary().child(format!("{photos} photos")),
            IndexState::NoIndex => Tag::secondary().child("no index"),
            IndexState::NotConnected => Tag::secondary().child("unplugged"),
            IndexState::Unreadable(_) => Tag::danger().child("unreadable"),
        };

        div()
            .id(("folder", index))
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .py_3()
            .px_1()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.6))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w(px(0.))
                    .flex_1()
                    .child(
                        div()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(pip)
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2_5()
                                    .child(div().font_semibold().text_sm().child(title))
                                    .child(status_tag)
                                    .children(is_external.then(|| {
                                        let medium_label = match row.medium {
                                            DiskMedium::Ssd => "SSD",
                                            DiskMedium::Hdd => "HDD",
                                            DiskMedium::Unknown => "DISK",
                                        };
                                        Tag::secondary().child(medium_label)
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(where_line),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted.opacity(0.8))
                                    .child(hint),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .children(connected.then(|| {
                        Button::new(SharedString::from(format!("index-{index}")))
                            .small()
                            .with_variant(if indexed {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Primary
                            })
                            .icon(IconName::RefreshCw)
                            .label(action)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_index(index_path.clone(), cx);
                            }))
                    }))
                    .children((!is_external && connected).then(|| {
                        Button::new(SharedString::from(format!("reveal-f-{index}")))
                            .small()
                            .ghost()
                            .icon(IconName::Folder)
                            .tooltip("Reveal in Finder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = reveal_path(&reveal_path_val);
                                this.status = "Showing in Finder.".into();
                                cx.notify();
                            }))
                    }))
                    .children(remembered.then(|| {
                        Button::new(SharedString::from(format!("forget-{index}")))
                            .small()
                            .ghost()
                            .icon(IconName::Trash)
                            .tooltip("Remove from search list")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.forget(forget_path.clone(), cx);
                            }))
                    })),
            )
    }
}

impl Render for PhotoApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let top_bar = self.render_top_bar(window, cx);
        let banner = (self.busy).then(|| self.render_index_banner(cx));
        let body: AnyElement = match self.screen {
            Screen::Folders => self.folders_page(window, cx).into_any_element(),
            Screen::Search => self.search_page(window, cx).into_any_element(),
        };
        let bottom_bar = search_hint(&self.settings, self.status.clone(), window, cx);

        div()
            .id("loupe")
            .track_focus(&self.focus)
            .key_context(SEARCH)
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_left))
            .on_action(cx.listener(Self::on_move_right))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_reveal))
            .on_action(cx.listener(Self::on_grid))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_quit))
            .on_action(cx.listener(Self::on_copy))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(top_bar)
            .children(banner)
            .child(body)
            .child(bottom_bar)
    }
}

const SUGGESTED_QUERIES: &[&str] = &[
    "wet street at night",
    "golden hour mountains",
    "dog in the grass",
    "85mm portrait",
    "kitchen window light",
    "red taillights",
];

fn ready_line(rows: &[FolderRow]) -> String {
    let labels: Vec<&str> = rows
        .iter()
        .filter(|row| matches!(row.index, IndexState::Ready { .. }))
        .map(|row| row.label.as_str())
        .collect();
    if labels.is_empty() {
        "No indexes active".into()
    } else {
        format!("{} active: {}", labels.len(), labels.join(", "))
    }
}

fn shortcut_label(settings: &GuiSettings) -> String {
    parse_chord(&settings.shortcut)
        .map(|chord| display_chord(&chord))
        .unwrap_or_else(|_| settings.shortcut.clone())
}

fn folder_title(row: &FolderRow) -> String {
    if row.kind == DiskKind::External {
        return row.label.clone();
    }
    row.path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| row.label.clone())
}

#[allow(dead_code)]
fn folder_status(row: &FolderRow) -> String {
    match &row.index {
        IndexState::Ready { photos, .. } => format!("{photos} photos"),
        IndexState::NoIndex => "Not indexed".into(),
        IndexState::NotConnected => "Not connected".into(),
        IndexState::Unreadable(_) => "Can't read the index".into(),
    }
}

fn folder_hint(row: &FolderRow) -> String {
    let line = match &row.index {
        IndexState::Ready { .. } if row.kind == DiskKind::External => "Search includes this disk.",
        IndexState::Ready { .. } if row.remembered => "Search includes this folder.",
        IndexState::Ready { .. } => "Included because an index is already here.",
        IndexState::NoIndex if row.kind == DiskKind::External => {
            "Index writes the catalog on this disk."
        }
        IndexState::NoIndex => "Index writes the catalog in this folder.",
        IndexState::NotConnected => "Plug it in to search the index on the disk.",
        IndexState::Unreadable(_) => "The catalog is on disk, but this Mac could not open it.",
    };
    match (row.kind, row.medium) {
        (DiskKind::External, DiskMedium::Hdd) => format!("{line} Hard disk."),
        (DiskKind::External, DiskMedium::Ssd) => format!("{line} Solid-state disk."),
        _ => line.to_string(),
    }
}

fn short_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        if let Ok(rest) = path.strip_prefix(home) {
            let rest = rest.display().to_string();
            if rest.is_empty() {
                return "~".into();
            }
            return format!("~/{rest}");
        }
    }
    path.display().to_string()
}

fn file_name(hit: &RankedHit) -> String {
    hit.path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| hit.path.display().to_string())
}

#[allow(dead_code)]
fn hit_meta(hit: &RankedHit) -> String {
    let mut line = hit.label.clone();
    if let Some(camera) = &hit.camera_model {
        line.push_str("  ");
        line.push_str(camera);
    }
    if let Some(mm) = hit.focal_length_mm {
        line.push_str(&format!("  {mm:.0} mm"));
    }
    line
}

fn thumb_box(path: Option<&std::path::Path>, width: f32, height: f32, cx: &App) -> AnyElement {
    let frame = div()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .rounded(px(2.))
        .bg(rgb(0x0E0C0A));
    if let Some(p) = path.filter(|p| p.is_file()) {
        frame
            .child(
                img(Arc::<std::path::Path>::from(p.to_path_buf()))
                    .w(px(width))
                    .h(px(height))
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element()
    } else {
        frame
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground.opacity(0.6))
            .child(IconName::Image)
            .into_any_element()
    }
}

#[allow(dead_code)]
fn index_strip(note: SharedString) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            Progress::new("index-progress")
                .loading(true)
                .accessibility_label("Indexing photos"),
        )
        .child(div().text_sm().child(note))
}

fn index_sentence(folder: &str, live: &IndexLive) -> String {
    if live.file.is_empty() {
        return format!("Indexing {folder}. Looking through the folder.");
    }
    let name = live.file.rsplit('/').next().unwrap_or(live.file.as_str());
    let mut line = format!(
        "Indexing {folder}. {} new, {} unchanged.",
        live.embedded, live.unchanged
    );
    if live.moved > 0 {
        line.push_str(&format!(" {} moved.", live.moved));
    }
    line.push(' ');
    line.push_str(name);
    line
}

fn search_hint(
    _settings: &GuiSettings,
    status_text: SharedString,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px_5()
        .py_1_5()
        .border_t_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(status_text)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&MoveLeft, SEARCH, "←", window))
                        .child(key_chip(&MoveRight, SEARCH, "→", window))
                        .child(key_chip(&MoveUp, SEARCH, "↑", window))
                        .child(key_chip(&MoveDown, SEARCH, "↓", window))
                        .child("navigate"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&OpenResult, SEARCH, "enter", window))
                        .child("open"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&RevealResult, SEARCH, "secondary-enter", window))
                        .child("Finder"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&CopyResult, SEARCH, "secondary-c", window))
                        .child("copy"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&ToggleGrid, SEARCH, "secondary-o", window))
                        .child("view"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&Dismiss, SEARCH, "escape", window))
                        .child("hide"),
                ),
        )
}

fn screen_tab(
    id: &'static str,
    label: &'static str,
    active: bool,
    paper: Hsla,
    mute: Hsla,
    safelight: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .pb(px(6.))
        .border_b_2()
        .border_color(if active { safelight } else { safelight.opacity(0.) })
        .text_sm()
        .text_color(if active { paper } else { mute })
        .cursor_pointer()
        .child(label)
        .on_click(on_click)
}

fn key_chip(action: &dyn Action, context: &str, fallback: &str, window: &Window) -> AnyElement {
    if let Some(kbd) = Kbd::binding_for_action(action, Some(context), window) {
        return kbd.outline().into_any_element();
    }
    match Keystroke::parse(fallback) {
        Ok(stroke) => Kbd::new(stroke).outline().into_any_element(),
        Err(_) => div().child(fallback.to_string()).into_any_element(),
    }
}

enum RowStep {
    Move(usize),
    Stay,
    Unknown,
}

fn item_center(bounds: Bounds<Pixels>) -> (f32, f32) {
    (
        f32::from(bounds.left()) + f32::from(bounds.size.width) * 0.5,
        f32::from(bounds.top()) + f32::from(bounds.size.height) * 0.5,
    )
}

fn side_cache(photo: &std::path::Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(photo).ok()?;
    let stamp = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_secs())
        .unwrap_or(0);
    let key = blake3::hash(format!("{}:{}:{stamp}", photo.display(), meta.len()).as_bytes());
    Some(
        sources::state_dir()
            .join("gui-preview")
            .join(format!("{}.jpg", key.to_hex())),
    )
}

fn file_is_large(path: &std::path::Path, bytes: u64) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.len() >= bytes)
        .unwrap_or(false)
}

fn hotkey_pressed() -> bool {
    let mut pressed = false;
    while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
        if event.state == HotKeyState::Pressed {
            pressed = true;
        }
    }
    pressed
}

fn install_shortcut(chord: &Chord) -> Result<()> {
    let hotkey = to_hotkey(chord)?;
    HOTKEYS.with(|slot| *slot.borrow_mut() = None);
    let manager = GlobalHotKeyManager::new().map_err(|err| anyhow!("could not install a shortcut. {err}"))?;
    manager
        .register(hotkey)
        .map_err(|err| anyhow!("could not register the shortcut. {err}"))?;
    HOTKEYS.with(|slot| *slot.borrow_mut() = Some(manager));
    Ok(())
}

fn to_hotkey(chord: &Chord) -> Result<HotKey> {
    let mut modifiers = Modifiers::empty();
    if chord.shift {
        modifiers |= Modifiers::SHIFT;
    }
    if chord.ctrl {
        modifiers |= Modifiers::CONTROL;
    }
    if chord.alt {
        modifiers |= Modifiers::ALT;
    }
    if chord.super_key {
        modifiers |= Modifiers::SUPER;
    }
    Ok(HotKey::new(Some(modifiers), chord_code(chord.key)?))
}

fn chord_code(key: ChordKey) -> Result<Code> {
    Ok(match key {
        ChordKey::Space => Code::Space,
        ChordKey::Letter(ch) => letter_code(ch)?,
        ChordKey::Function(number) => function_code(number)?,
    })
}

fn letter_code(ch: char) -> Result<Code> {
    Ok(match ch {
        'a' => Code::KeyA,
        'b' => Code::KeyB,
        'c' => Code::KeyC,
        'd' => Code::KeyD,
        'e' => Code::KeyE,
        'f' => Code::KeyF,
        'g' => Code::KeyG,
        'h' => Code::KeyH,
        'i' => Code::KeyI,
        'j' => Code::KeyJ,
        'k' => Code::KeyK,
        'l' => Code::KeyL,
        'm' => Code::KeyM,
        'n' => Code::KeyN,
        'o' => Code::KeyO,
        'p' => Code::KeyP,
        'q' => Code::KeyQ,
        'r' => Code::KeyR,
        's' => Code::KeyS,
        't' => Code::KeyT,
        'u' => Code::KeyU,
        'v' => Code::KeyV,
        'w' => Code::KeyW,
        'x' => Code::KeyX,
        'y' => Code::KeyY,
        'z' => Code::KeyZ,
        _ => anyhow::bail!("unsupported letter"),
    })
}

fn function_code(number: u8) -> Result<Code> {
    Ok(match number {
        1 => Code::F1,
        2 => Code::F2,
        3 => Code::F3,
        4 => Code::F4,
        5 => Code::F5,
        6 => Code::F6,
        7 => Code::F7,
        8 => Code::F8,
        9 => Code::F9,
        10 => Code::F10,
        11 => Code::F11,
        12 => Code::F12,
        13 => Code::F13,
        14 => Code::F14,
        15 => Code::F15,
        16 => Code::F16,
        17 => Code::F17,
        18 => Code::F18,
        19 => Code::F19,
        20 => Code::F20,
        21 => Code::F21,
        22 => Code::F22,
        23 => Code::F23,
        24 => Code::F24,
        _ => anyhow::bail!("unsupported function key"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource;

    #[core::prelude::v1::test]
    fn test_all_assets_contains_required_icons() {
        let assets = gpui_kit::assets::AllAssets;
        for path in &[
            "icons/camera.svg",
            "icons/layout-grid.svg",
            "icons/layout-list.svg",
            "icons/folder-plus.svg",
            "icons/images.svg",
            "icons/sparkles.svg",
            "icons/search.svg",
            "icons/hard-drive.svg",
            "icons/folder.svg",
            "icons/trash.svg",
            "icons/copy.svg",
            "icons/external-link.svg",
            "icons/refresh-cw.svg",
        ] {
            assert!(
                assets.load(path).expect("load result").is_some(),
                "missing icon asset: {}",
                path
            );
        }
    }

    #[core::prelude::v1::test]
    fn test_copy_file_on_image() {
        let img_path = std::env::temp_dir().join("photo_engine_test_copy.png");
        let png_bytes = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&img_path, png_bytes).expect("write png");
        let result = copy_file(&img_path);
        let _ = std::fs::remove_file(&img_path);
        assert!(result.is_ok(), "copy_file should succeed on valid image: {:?}", result);
    }
}
