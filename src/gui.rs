use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex, StyledExt};
use gpui_kit::component::alert::Alert;
use gpui_kit::component::carousel::{
    Carousel, CarouselContent, CarouselItem, CarouselPagination, CarouselPaginationItem,
    CarouselState,
};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::empty::{
    Empty as EmptyState, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia,
    EmptyMediaVariant, EmptyTitle,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::scroll::{ScrollableElement, Scrollbar};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{
    v_virtual_list, ActiveTheme, Sizable, VirtualListScrollHandle,
};
use gpui_apple_hig::{
    apply_apple_theme, format_mac_shortcut, MacAccentColor, MacBadge,
    MacButton, MacButtonVariant, MacKbd, MacMenuItem, MacPathControl,
    MacPopupButton, MacProgress, MacPullDownButton, MacSearchField, MacSegment,
    MacSegmentedControl, MacSettingsGroup, MacSettingsRow, MacSpinner, MacSpinnerSize,
    MacStatusBar, MacSwitch, MacToolbar, MacTypographyExt,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::app_model::{
    self, check_update, copy_file, display_chord, display_thumb, folder_rows, format_chord,
    index_failure, index_folder_live, index_summary, is_theme_dark, open_path, open_url,
    parse_chord, release_url, reveal_path, save_settings, search_photos, system_is_dark,
    update_is_due, write_preview, Chord, ChordKey, DiskKind, FolderRow, GuiSettings, IndexLive,
    IndexState, UpdateReport, APP_VERSION, THEME_DARK, THEME_DARKROOM, THEME_LIGHT, THEME_PAPER,
    THEME_SYSTEM,
};
use crate::documents;
use crate::embedder::MODEL_ID;
use crate::gemma::{self, FetchLive, GemmaEmbedder};
use crate::indexer;
use crate::lookup::RankedHit;
use crate::menu_bar::{self, TrayCommand};
use crate::player::{format_duration, query_file_duration, MediaPlayer};
use crate::sources::{self, DiskMedium};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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
        RefreshPreview,
        CopyResult,
        QuitApp,
        FocusSearch,
        OpenSettings,
    ]
);

const SEARCH: &str = "PhotoSearch";
const RESULTS: &str = "PhotoResults";
const PRODUCT: &str = "Loupe";
const SEARCH_LIMIT: usize = 48;
const WATCH_DEBOUNCE: Duration = Duration::from_secs(2);
const WATCH_PUMP: Duration = Duration::from_millis(400);
const PAGE_INSET: f32 = 20.;
const TILE_GAP: f32 = 12.;
const SHEET_PAD: f32 = 24.;
const SHEET_MIN_LAYOUT: f32 = 80.;
const SHEET_FALLBACK_INNER: f32 = 880.;
const SHEET_MIN_INNER: f32 = 300.;
const TITLE_CHROME: f32 = 52.;
const SEARCH_BAR_HEIGHT: f32 = 52.;
const STATUS_BAR_HEIGHT: f32 = 28.;
const INDEX_BANNER_HEIGHT: f32 = 40.;
const LIST_ROW_HEIGHT: f32 = 88.;
const TILE_CAPTION: f32 = 54.;
const PREVIEW_WIDTH: f32 = 440.;

thread_local! {
    static HOTKEYS: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

enum Screen {
    Search,
    Settings,
}

pub fn run() -> Result<()> {
    application().with_assets(assets::AllAssets).run(|cx| {
        init(cx);
        cx.set_activation_policy(ActivationPolicy::Accessory);
        let settings = app_model::load_settings();
        apply_named_theme(&settings.theme, &settings.accent, cx);
        bind_keys(cx);
        cx.set_menus([Menu::new("Loupe").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Hide Loupe", Dismiss),
        ])]);
        // Input swallows cmd+, as text. Intercept before the field inserts a comma.
        let settings_chord = cx.intercept_keystrokes(|event, window, cx| {
            let ks = &event.keystroke;
            if !ks.modifiers.platform
                || ks.modifiers.shift
                || ks.modifiers.alt
                || ks.modifiers.control
            {
                return;
            }
            if ks.key == "," || ks.key == "comma" {
                window.dispatch_action(Box::new(OpenSettings), cx);
                cx.stop_propagation();
            }
        });
        std::mem::forget(settings_chord);
        cx.on_action(|_: &Dismiss, cx| cx.hide());
        cx.on_action(|_: &QuitApp, cx| cx.hide());
        match menu_bar::install_tray() {
            Ok(()) => note("menu bar ready"),
            Err(err) => eprintln!("menu bar: {err}"),
        }
        let bounds = Bounds::centered(None, size(px(1240.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(860.), px(560.))),
            window_background: WindowBackgroundAppearance::Blurred,
            titlebar: Some(TitlebarOptions {
                title: Some("Loupe".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(20.0), px(19.0))),
            }),
            app_owns_titlebar_drag: false,
            focus: true,
            show: true,
            ..Default::default()
        };
        match open_window(options, cx, |window, cx| {
            cx.new(|cx| PhotoApp::new(window, cx))
        }) {
            Ok(_) => {
                note("window open");
                note("idle");
            }
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

pub fn accent_from_str(s: &str) -> MacAccentColor {
    match s.trim().to_ascii_lowercase().as_str() {
        "purple" => MacAccentColor::Purple,
        "pink" => MacAccentColor::Pink,
        "red" => MacAccentColor::Red,
        "orange" => MacAccentColor::Orange,
        "yellow" => MacAccentColor::Yellow,
        "green" => MacAccentColor::Green,
        "graphite" => MacAccentColor::Graphite,
        _ => MacAccentColor::Blue,
    }
}

fn apply_named_theme(name: &str, accent_name: &str, cx: &mut App) {
    let is_dark = is_theme_dark(name);
    let accent = accent_from_str(accent_name);
    apply_apple_theme(is_dark, accent, cx);
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
        KeyBinding::new("secondary-q", Dismiss, None),
        KeyBinding::new("secondary-q", Dismiss, Some(SEARCH)),
        KeyBinding::new("secondary-q", Dismiss, Some(RESULTS)),
        KeyBinding::new("space", RefreshPreview, Some(RESULTS)),
        KeyBinding::new("secondary-c", CopyResult, Some(RESULTS)),
        KeyBinding::new("secondary-c", CopyResult, Some(SEARCH)),
        KeyBinding::new("secondary-f", FocusSearch, Some(SEARCH)),
        KeyBinding::new("/", FocusSearch, Some(RESULTS)),
        KeyBinding::new("secondary-comma", OpenSettings, None),
        KeyBinding::new("secondary-comma", OpenSettings, Some(SEARCH)),
        KeyBinding::new("secondary-comma", OpenSettings, Some(RESULTS)),
        // macOS reports the key as "," not "comma"
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-,", OpenSettings, Some(SEARCH)),
        KeyBinding::new("secondary-,", OpenSettings, Some(RESULTS)),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HitSortOrder {
    #[default]
    BestMatch,
    NameAsc,
    NameDesc,
}

struct PhotoApp {
    query_input: Entity<InputState>,
    feed_input: Entity<InputState>,
    shortcut_input: Entity<InputState>,
    query: String,
    hits: Vec<RankedHit>,
    sort_order: HitSortOrder,
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
    preview_for: Option<PathBuf>,
    rows: Vec<FolderRow>,
    settings: GuiSettings,
    shortcut_error: String,
    update_line: String,
    release_url: Option<String>,
    busy: bool,
    indexing_path: Option<PathBuf>,
    index_live: IndexLive,
    fetching: bool,
    fetch_note: SharedString,
    fetch_frac: f32,
    generation: u64,
    embedder: Arc<GemmaEmbedder>,
    focus: FocusHandle,
    results_focus: FocusHandle,
    hits_scroll: VirtualListScrollHandle,
    welcome_carousel: Entity<CarouselState>,
    search_task: Option<Task<()>>,
    preview_task: Option<Task<()>>,
    thumb_task: Option<Task<()>>,
    index_task: Option<Task<()>>,
    _index_watch: Option<Task<()>>,
    fetch_task: Option<Task<()>>,
    _fetch_watch: Option<Task<()>>,
    update_task: Option<Task<()>>,
    picker_task: Option<Task<()>>,
    _hotkey_task: Task<()>,
    _carousel_task: Task<()>,
    _watch_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    parked: bool,
    folder_watch: Option<RecommendedWatcher>,
    watch_dirty: Arc<Mutex<HashMap<PathBuf, Instant>>>,
    watch_roots: Vec<PathBuf>,
    pending_index: HashSet<PathBuf>,
    last_sweep_unix: u64,
    did_catch_up: bool,
    player: MediaPlayer,
    player_tick_task: Option<Task<()>>,
}

impl PhotoApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = app_model::load_settings();
        let query_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Describe a photo, a file, a clip")
        });
        let feed_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(app_model::DEFAULT_UPDATE_FEED)
        });
        let shortcut_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(settings.shortcut.clone())
        });
        let saved_shortcut = settings.shortcut.clone();
        shortcut_input.update(cx, |state, cx| {
            state.replace_all(saved_shortcut, window, cx);
        });
        let saved_feed = settings.update_feed.clone();
        feed_input.update(cx, |state, cx| {
            state.replace_all(saved_feed, window, cx);
        });
        let closer = cx.weak_entity();
        window.on_window_should_close(cx, move |_window, cx| {
            let _ = closer.update(cx, |this, cx| this.hide_to_tray(cx));
            false
        });

        let subscription = cx.subscribe_in(&query_input, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                this.schedule_search(cx);
            }
        });
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            let target = window
                .focus_lost_restore_target(cx)
                .unwrap_or_else(|| this.query_input.focus_handle(cx));
            target.focus(window, cx);
        });

        let rows = folder_rows().unwrap_or_default();
        let status = if rows
            .iter()
            .any(|row| matches!(row.index, IndexState::Ready { .. }))
        {
            "Describe a photo."
        } else {
            "No index yet. Open Settings and index a folder or a mounted disk."
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
        let welcome_carousel = cx.new(|_| {
            CarouselState::new(SUGGESTED_QUERIES.len()).with_looping(true)
        });
        let due = update_is_due(&settings);
        let hotkey_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let chord = hotkey_pressed();
                let tray = menu_bar::poll_tray();
                if !chord && tray.is_none() {
                    continue;
                }
                let _ = this.update_in(cx, |view, window, cx| {
                    if chord {
                        view.present(window, cx);
                        return;
                    }
                    match tray {
                        Some(TrayCommand::Show) => view.present(window, cx),
                        Some(TrayCommand::Settings) => {
                            view.on_open_settings(&OpenSettings, window, cx);
                            view.reveal_window(window, cx);
                        }
                        Some(TrayCommand::Quit) => cx.quit(),
                        None => {}
                    }
                });
            }
        });
        let mut app = Self {
            query_input,
            feed_input,
            shortcut_input,
            query: String::new(),
            hits: Vec::new(),
            sort_order: HitSortOrder::BestMatch,
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
            indexing_path: None,
            index_live: IndexLive::default(),
            fetching: false,
            fetch_note: SharedString::default(),
            fetch_frac: 0.,
            generation: 0,
            embedder: Arc::new(GemmaEmbedder::new()),
            focus,
            results_focus,
            hits_scroll: VirtualListScrollHandle::new(),
            search_task: None,
            preview_task: None,
            thumb_task: None,
            index_task: None,
            _index_watch: None,
            fetch_task: None,
            _fetch_watch: None,
            update_task: None,
            picker_task: None,
            welcome_carousel,
            parked: false,
            _hotkey_task: hotkey_task,
            _carousel_task: cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(4))
                        .await;
                    let _ = this.update(cx, |view, cx| {
                        if view.parked {
                            return;
                        }
                        if matches!(view.screen, Screen::Search)
                            && view.query.trim().is_empty()
                            && view.hits.is_empty()
                        {
                            view.welcome_carousel.update(cx, |state, cx| {
                                let _ = state.select_next(cx);
                            });
                        }
                    });
                }
            }),
            _watch_task: None,
            _subscriptions: vec![subscription, focus_lost],
            folder_watch: None,
            watch_dirty: Arc::new(Mutex::new(HashMap::new())),
            watch_roots: Vec::new(),
            pending_index: HashSet::new(),
            last_sweep_unix: app_model::unix_now(),
            did_catch_up: false,
            player: MediaPlayer::new(),
            player_tick_task: None,
        };
        if due {
            app.spawn_update(cx);
        }
        app.start_model_fetch(cx);
        app.start_folder_watch(cx);
        app.queue_catch_up(cx);
        app.queue_stale_catalogs(cx);
        if app.settings.open_at_login {
            if let Err(err) = menu_bar::set_open_at_login(true) {
                eprintln!("open at login: {err}");
            }
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update_in(cx, |view, window, _cx| {
                if view.parked {
                    return;
                }
                let vp = window.viewport_size();
                let b = view.hits_scroll.bounds();
                let line = format!(
                    "layout viewport={:.0}x{:.0} welcome origin={:.0},{:.0} size={:.0}x{:.0}",
                    f32::from(vp.width),
                    f32::from(vp.height),
                    f32::from(b.origin.x),
                    f32::from(b.origin.y),
                    f32::from(b.size.width),
                    f32::from(b.size.height),
                );
                note(&line);
                let _ = std::fs::write("/tmp/loupe-layout.txt", format!("{line}\n"));
            });
        })
        .detach();
        app
    }

    fn drop_surfaces(&mut self) {
        self.stop_media_playback_silent();
        self.search_task = None;
        self.preview_task = None;
        self.thumb_task = None;
        self.thumbs.clear();
        self.preview = None;
        self.preview_for = None;
    }

    fn hide_to_tray(&mut self, cx: &mut Context<Self>) {
        self.parked = true;
        self.generation = self.generation.wrapping_add(1);
        self.drop_surfaces();
        cx.hide();
        cx.notify();
    }

    fn present(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.theme == THEME_SYSTEM {
            let want_dark = system_is_dark();
            if cx.theme().mode.is_dark() != want_dark {
                apply_named_theme(&self.settings.theme, &self.settings.accent, cx);
            }
        }
        self.parked = false;
        self.screen = Screen::Search;
        self.reveal_window(window, cx);
        self.query_input.focus_handle(cx).focus(window, cx);
        if !self.query.trim().is_empty() {
            if self.hits.is_empty() {
                self.schedule_search(cx);
            } else {
                self.warm_thumbs(cx);
                self.queue_side_preview(cx);
            }
        }
        cx.notify();
    }

    fn reveal_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.parked = false;
        cx.activate(true);
        window.activate_window();
        cx.notify();
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        if self.parked {
            return;
        }
        self.stop_media_playback_silent();
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
            self.status = "Describe a photo.".into();
            self.thumbs.clear();
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
                .background_spawn(async move { search_photos(embedder.as_ref(), &query, SEARCH_LIMIT) })
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                match outcome {
                    Ok(outcome) => {
                        view.hits = outcome.hits;
                        view.apply_sort();
                        view.selected = 0;
                        view.index_line = outcome.index_line.into();
                        view.notes = outcome.notes;
                        view.errors = outcome.errors;
                        view.retain_thumbs();
                        view.status = search_status(&view.hits, &view.notes, &view.index_line);
                    }
                    Err(err) => {
                        view.hits.clear();
                        view.selected = 0;
                        view.thumbs.clear();
                        view.status = format!("Search failed. {err}").into();
                    }
                }
                view.warm_thumbs(cx);
                view.scroll_selected_into_view();
                view.queue_side_preview(cx);
                cx.notify();
            });
        }));
    }

    fn retain_thumbs(&mut self) {
        let keep: HashSet<PathBuf> = self.hits.iter().map(|hit| hit.path.clone()).collect();
        self.thumbs.retain(|path, _| keep.contains(path));
    }

    fn warm_thumbs(&mut self, cx: &mut Context<Self>) {
        if self.parked {
            return;
        }
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
        self.sheet_metrics().cols as isize
    }

    fn sheet_metrics(&self) -> SheetMetrics {
        SheetMetrics::from_width(f32::from(self.hits_scroll.bounds().size.width))
    }

    fn scroll_selected_into_view(&self) {
        if self.hits.is_empty() {
            return;
        }
        if self.grid {
            let cols = self.columns().max(1) as usize;
            self.hits_scroll
                .scroll_to_item(self.selected / cols, ScrollStrategy::Center);
        } else {
            self.hits_scroll
                .scroll_to_item(self.selected, ScrollStrategy::Center);
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
            let step = if self.grid {
                self.columns().max(1)
            } else {
                1
            };
            (self.selected as isize + vertical.signum() * step).clamp(0, last) as usize
        } else {
            (self.selected as isize + horizontal).clamp(0, last) as usize
        };
        if next == self.selected {
            self.scroll_selected_into_view();
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
        let target = index.min(self.hits.len() - 1);
        if self.selected != target {
            self.stop_media_playback_silent();
        }
        self.selected = target;
        self.scroll_selected_into_view();
        self.results_focus.focus(window, cx);
        self.queue_side_preview(cx);
        cx.notify();
    }

    fn queue_side_preview(&mut self, cx: &mut Context<Self>) {
        if self.parked {
            return;
        }
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

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            return;
        }
        if self.hits.get(self.selected).is_none() {
            self.status = "Move to a photo first.".into();
            cx.notify();
            return;
        }
        self.preview_for = None;
        self.queue_side_preview(cx);
        cx.notify();
    }

    fn toggle_media_playback(&mut self, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(self.selected) else {
            return;
        };
        let path = hit.path.clone();
        let fname = file_name(hit);
        let mime = documents::mime_for_path(&path);
        if !mime.is_some_and(documents::is_playable) {
            return;
        }

        if self.player.current_path() != Some(&path) {
            if !self.player.load(&path) {
                self.status = "Could not open media file for playback.".into();
                cx.notify();
                return;
            }
        }

        self.player.toggle_play_pause();
        if self.player.is_playing() {
            self.spawn_player_ticker(cx);
            self.status = format!("Playing {fname}").into();
        } else {
            self.player_tick_task = None;
            self.status = format!("Paused {fname}").into();
        }
        cx.notify();
    }

    #[allow(dead_code)]
    fn stop_media_playback(&mut self, cx: &mut Context<Self>) {
        if self.player.is_loaded() {
            self.player.stop();
            self.player_tick_task = None;
            cx.notify();
        }
    }

    fn stop_media_playback_silent(&mut self) {
        if self.player.is_loaded() {
            self.player.stop();
            self.player_tick_task = None;
        }
    }

    fn seek_media(&mut self, seconds: f64, cx: &mut Context<Self>) {
        if self.player.is_loaded() {
            self.player.seek(seconds);
            cx.notify();
        }
    }

    fn spawn_player_ticker(&mut self, cx: &mut Context<Self>) {
        self.player_tick_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let still_playing = this.update(cx, |view, cx| {
                    if view.parked || !view.player.is_playing() {
                        view.player_tick_task = None;
                        cx.notify();
                        false
                    } else {
                        cx.notify();
                        true
                    }
                });
                if still_playing.unwrap_or(false) == false {
                    break;
                }
            }
        }));
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
        self.indexing_path = Some(path.clone());
        self.index_live = IndexLive::default();
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
                            view.index_live = snap.clone();
                            view.index_note = index_sentence(&folder_watch, &snap).into();
                            if !view.parked {
                                cx.notify();
                            }
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
                view.indexing_path = None;
                view.index_live = IndexLive::default();
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
                view.sync_folder_watch();
                view.drain_pending_index(cx);
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

    fn start_folder_watch(&mut self, cx: &mut Context<Self>) {
        self.sync_folder_watch();
        self._watch_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(WATCH_PUMP).await;
                let _ = this.update(cx, |view, cx| view.pump_watch(cx));
            }
        }));
    }

    fn pump_watch(&mut self, cx: &mut Context<Self>) {
        self.sync_folder_watch();
        if self.fetching {
            return;
        }
        let now = Instant::now();
        let mut due = Vec::new();
        if self.settings.watch_folders {
            if let Ok(mut dirty) = self.watch_dirty.lock() {
                dirty.retain(|path, at| {
                    if now.saturating_duration_since(*at) >= WATCH_DEBOUNCE {
                        due.push(path.clone());
                        false
                    } else {
                        true
                    }
                });
            }
        }
        let hours = self.settings.index_sweep_hours;
        if hours > 0 {
            let elapsed = app_model::unix_now().saturating_sub(self.last_sweep_unix);
            if elapsed >= hours.saturating_mul(3600) {
                self.last_sweep_unix = app_model::unix_now();
                for row in &self.rows {
                    if matches!(row.index, IndexState::Ready { .. })
                        && sources::path_on_live_volume(&row.path)
                    {
                        due.push(row.path.clone());
                    }
                }
            }
        }
        due.sort();
        due.dedup();
        for path in due {
            self.queue_index(path, cx);
        }
    }

    fn sync_folder_watch(&mut self) {
        if !self.settings.watch_folders {
            self.folder_watch = None;
            self.watch_roots.clear();
            if let Ok(mut dirty) = self.watch_dirty.lock() {
                dirty.clear();
            }
            return;
        }
        let mut roots: Vec<PathBuf> = self
            .rows
            .iter()
            .filter(|row| {
                matches!(row.index, IndexState::Ready { .. })
                    && sources::path_on_live_volume(&row.path)
            })
            .map(|row| row.path.clone())
            .collect();
        roots.sort();
        roots.dedup();
        if self.folder_watch.is_some() && roots == self.watch_roots {
            return;
        }
        self.folder_watch = None;
        if roots.is_empty() {
            self.watch_roots.clear();
            return;
        }
        let dirty = Arc::clone(&self.watch_dirty);
        let watched = roots.clone();
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            let Ok(event) = res else {
                return;
            };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            for path in &event.paths {
                if indexer::path_is_catalog_noise(path) {
                    continue;
                }
                let Some(root) = watched.iter().find(|root| path.starts_with(root)) else {
                    continue;
                };
                if let Ok(mut map) = dirty.lock() {
                    map.insert(root.clone(), Instant::now());
                }
            }
        }) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };
        for root in &roots {
            let _ = watcher.watch(root, RecursiveMode::Recursive);
        }
        self.folder_watch = Some(watcher);
        self.watch_roots = roots;
    }

    fn queue_index(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.fetching || gemma::needs_fetch() {
            self.pending_index.insert(path);
            return;
        }
        if !sources::path_on_live_volume(&path) {
            return;
        }
        if self.indexing_path.as_ref() == Some(&path) || self.busy {
            self.pending_index.insert(path);
            return;
        }
        self.start_index(path, cx);
    }

    fn drain_pending_index(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let next = self.pending_index.iter().next().cloned();
        if let Some(path) = next {
            self.pending_index.remove(&path);
            self.queue_index(path, cx);
        }
    }

    fn queue_catch_up(&mut self, cx: &mut Context<Self>) {
        if self.did_catch_up {
            return;
        }
        if self.fetching || gemma::needs_fetch() {
            return;
        }
        if !self.settings.watch_folders && self.settings.index_sweep_hours == 0 {
            return;
        }
        self.did_catch_up = true;
        self.last_sweep_unix = app_model::unix_now();
        let paths: Vec<PathBuf> = self
            .rows
            .iter()
            .filter(|row| {
                matches!(row.index, IndexState::Ready { .. })
                    && sources::path_on_live_volume(&row.path)
            })
            .map(|row| row.path.clone())
            .collect();
        for path in paths {
            self.queue_index(path, cx);
        }
    }

    /// Folders whose catalog is still on the previous embedder. Search skips
    /// them until Index finishes. This runs even when watch and sweep are off.
    fn queue_stale_catalogs(&mut self, cx: &mut Context<Self>) {
        if self.fetching || gemma::needs_fetch() {
            return;
        }
        let paths: Vec<PathBuf> = self
            .rows
            .iter()
            .filter(|row| match &row.index {
                IndexState::Ready { model, .. } => {
                    model.as_deref() != Some(MODEL_ID)
                        && sources::path_on_live_volume(&row.path)
                }
                _ => false,
            })
            .map(|row| row.path.clone())
            .collect();
        for path in paths {
            self.queue_index(path, cx);
        }
    }

    fn set_watch_folders(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.settings.watch_folders = enabled;
        if let Err(err) = save_settings(&self.settings) {
            self.status = err.to_string().into();
        }
        self.sync_folder_watch();
        cx.notify();
    }

    fn set_sweep_hours(&mut self, hours: u64, cx: &mut Context<Self>) {
        self.settings.index_sweep_hours = hours;
        self.last_sweep_unix = app_model::unix_now();
        if let Err(err) = save_settings(&self.settings) {
            self.status = err.to_string().into();
        }
        cx.notify();
    }

    fn start_model_fetch(&mut self, cx: &mut Context<Self>) {
        if !gemma::needs_fetch() {
            return;
        }
        self.fetching = true;
        self.fetch_note = "Downloading EmbeddingGemma 2…".into();
        self.fetch_frac = 0.;
        let live = Arc::new(Mutex::new(FetchLive::default()));
        let watch = Arc::clone(&live);
        self._fetch_watch = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let snap = watch.lock().ok().map(|slot| slot.clone());
                let keep = this
                    .update(cx, |view, cx| {
                        if !view.fetching {
                            return false;
                        }
                        if let Some(snap) = snap {
                            if snap.total > 0 {
                                view.fetch_frac =
                                    (snap.got as f32 / snap.total as f32) * 100.;
                                view.fetch_note = format!(
                                    "Downloading {}. {:.0} / {:.0} MB.",
                                    snap.phase,
                                    snap.got as f64 / 1_000_000.0,
                                    snap.total as f64 / 1_000_000.0
                                )
                                .into();
                            }
                            if !view.parked {
                                cx.notify();
                            }
                            if snap.done {
                                return false;
                            }
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        }));
        self.fetch_task = Some(cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { gemma::prepare_runtime(live) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.fetching = false;
                match outcome {
                    Ok(()) => {
                        view.fetch_frac = 100.;
                        view.fetch_note = "EmbeddingGemma 2 is on this Mac.".into();
                        view.status = "Model ready.".into();
                        view.queue_catch_up(cx);
                        view.queue_stale_catalogs(cx);
                    }
                    Err(err) => {
                        view.fetch_note = format!("Could not download the model. {err}").into();
                        view.status = "Model download failed.".into();
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn add_folder(&mut self, cx: &mut Context<Self>) {
        self.pick_folder(None, "Choose a folder of photos", cx);
    }

    fn pick_folder_on_disk(&mut self, start: std::path::PathBuf, cx: &mut Context<Self>) {
        self.pick_folder(
            Some(start),
            "Choose a folder of photos on this disk",
            cx,
        );
    }

    fn pick_folder(&mut self, start: Option<std::path::PathBuf>, title: &'static str, cx: &mut Context<Self>) {
        // NSOpenPanel runs a nested modal loop. Calling it while PhotoApp is
        // borrowed (the click listener) lets the 100 ms hotkey timer
        // borrow_mut the same RefCell and abort.
        cx.activate(true);
        self.picker_task = Some(cx.spawn(async move |this, cx| {
            let mut dialog = rfd::FileDialog::new().set_title(title);
            if let Some(path) = start {
                dialog = dialog.set_directory(path);
            }
            let Some(path) = dialog.pick_folder() else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if let Err(err) = sources::remember(&path) {
                    view.status = err.to_string().into();
                    cx.notify();
                    return;
                }
                view.reload_rows();
                view.start_index(path, cx);
            });
        }));
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
        self.settings.update_feed = app_model::DEFAULT_UPDATE_FEED.to_string();
        self.release_url = None;
        self.update_line = "Using the GitHub feed.".into();
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

    fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        let canonical = match name {
            THEME_LIGHT | THEME_PAPER => THEME_LIGHT,
            THEME_DARK | THEME_DARKROOM => THEME_DARK,
            _ => THEME_SYSTEM,
        };
        if self.settings.theme == canonical {
            return;
        }
        self.settings.theme = canonical.to_string();
        apply_named_theme(canonical, &self.settings.accent, cx);
        if let Err(err) = save_settings(&self.settings) {
            self.status = err.to_string().into();
        }
        cx.notify();
    }

    fn set_accent(&mut self, accent: &str, cx: &mut Context<Self>) {
        if self.settings.accent == accent {
            return;
        }
        self.settings.accent = accent.to_string();
        apply_named_theme(&self.settings.theme, accent, cx);
        if let Err(err) = save_settings(&self.settings) {
            self.status = err.to_string().into();
        }
        cx.notify();
    }

    fn apply_sort(&mut self) {
        match self.sort_order {
            HitSortOrder::BestMatch => {
                self.hits.sort_by(|a, b| b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal));
            }
            HitSortOrder::NameAsc => {
                self.hits.sort_by(|a, b| {
                    let name_a = file_name(a).to_lowercase();
                    let name_b = file_name(b).to_lowercase();
                    name_a.cmp(&name_b)
                });
            }
            HitSortOrder::NameDesc => {
                self.hits.sort_by(|a, b| {
                    let name_a = file_name(a).to_lowercase();
                    let name_b = file_name(b).to_lowercase();
                    name_b.cmp(&name_a)
                });
            }
        }
    }

    fn set_sort_order(&mut self, order: HitSortOrder, cx: &mut Context<Self>) {
        if self.sort_order == order {
            return;
        }
        self.sort_order = order;
        self.apply_sort();
        self.selected = 0;
        self.queue_side_preview(cx);
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
            self.scroll_selected_into_view();
            self.results_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn on_quit(&mut self, _: &QuitApp, _: &mut Window, cx: &mut Context<Self>) {
        self.hide_to_tray(cx);
    }

    fn on_dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        self.hide_to_tray(cx);
    }

    fn on_preview(&mut self, _: &RefreshPreview, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(hit) = self.hits.get(self.selected) {
            let mime = documents::mime_for_path(&hit.path);
            if mime.is_some_and(documents::is_playable) {
                self.toggle_media_playback(cx);
                return;
            }
        }
        self.refresh_preview(cx);
    }

    fn on_copy(&mut self, _: &CopyResult, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(cx);
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.screen, Screen::Search) {
            self.screen = Screen::Search;
        }
        self.query_input.focus_handle(cx).focus(window, cx);
        self.query_input.update(cx, |state, cx| {
            state.select_all(window, cx);
        });
        cx.notify();
    }

    fn on_open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.theme == THEME_SYSTEM {
            let want_dark = system_is_dark();
            if cx.theme().mode.is_dark() != want_dark {
                apply_named_theme(&self.settings.theme, &self.settings.accent, cx);
            }
        }
        self.parked = false;
        self.screen = Screen::Settings;
        self.reveal_window(window, cx);
        self.reload_rows();
        let vp = window.viewport_size();
        let inner = Self::settings_inner_width(window);
        let line = format!(
            "settings viewport={:.0}x{:.0} inner={:.0}",
            f32::from(vp.width),
            f32::from(vp.height),
            inner
        );
        note(&line);
        let _ = std::fs::write("/tmp/loupe-layout.txt", format!("{line}\n"));
        cx.notify();
    }

    fn set_open_at_login(&mut self, enabled: bool, cx: &mut Context<Self>) {
        match menu_bar::set_open_at_login(enabled) {
            Ok(()) => {
                self.settings.open_at_login = enabled;
                if enabled {
                    self.status = "Loupe will open at login.".into();
                } else {
                    self.status = "Loupe will stay closed until you open it.".into();
                }
            }
            Err(err) => {
                self.settings.open_at_login = menu_bar::open_at_login_installed();
                self.status = err.to_string().into();
            }
        }
        if let Err(err) = save_settings(&self.settings) {
            self.status = err.to_string().into();
        }
        cx.notify();
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
            format!("{} results", self.hits.len())
        } else if ready_count > 0 {
            format!("{ready_count} photos")
        } else {
            String::new()
        };

        let mut toolbar = MacToolbar::new("loupe-toolbar");

        if is_search {
            toolbar = toolbar.leading(
                div()
                    .id("wordmark")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .cursor_pointer()
                    .child(
                        svg()
                            .path("icons/search.svg")
                            .size(px(14.0))
                            .text_color(cx.theme().primary),
                    )
                    .child(
                        div()
                            .mac_headline()
                            .text_color(cx.theme().foreground)
                            .child(PRODUCT),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.present(window, cx);
                    })),
            );

            toolbar = toolbar.center(
                MacSegmentedControl::new("view-mode")
                    .segment(
                        MacSegment::new("grid")
                            .icon(IconName::LayoutGrid)
                            .tooltip("Contact sheet (⌘O)"),
                    )
                    .segment(
                        MacSegment::new("list")
                            .icon(IconName::LayoutList)
                            .tooltip("List (⌘O)"),
                    )
                    .selected_index(if self.grid { 0 } else { 1 })
                    .on_change(cx.listener(|this, idx: &usize, window, cx| {
                        let want_grid = *idx == 0;
                        if want_grid != this.grid {
                            this.on_grid(&ToggleGrid, window, cx);
                        }
                    })),
            );

            if !self.hits.is_empty() {
                let sort_idx = match self.sort_order {
                    HitSortOrder::BestMatch => 0,
                    HitSortOrder::NameAsc => 1,
                    HitSortOrder::NameDesc => 2,
                };
                let sort_btn = MacPopupButton::new("sort-order-btn")
                    .item(MacMenuItem::new("sort-best", "Best Match").icon(IconName::Sparkles))
                    .item(MacMenuItem::new("sort-az", "Name (A → Z)"))
                    .item(MacMenuItem::new("sort-za", "Name (Z → A)"))
                    .selected_index(sort_idx)
                    .anchor(Anchor::TopRight)
                    .on_change(cx.listener(|this, idx: &usize, _, cx| {
                        let order = match *idx {
                            1 => HitSortOrder::NameAsc,
                            2 => HitSortOrder::NameDesc,
                            _ => HitSortOrder::BestMatch,
                        };
                        this.set_sort_order(order, cx);
                    }));
                toolbar = toolbar.trailing(sort_btn);

                let selected_hit = self.hits.get(self.selected).cloned();
                let mut action_btn = MacPullDownButton::new("toolbar-actions", "Action")
                    .icon(IconName::Sparkles)
                    .anchor(Anchor::TopRight)
                    .disabled(selected_hit.is_none());

                if let Some(hit) = selected_hit {
                    let is_media = documents::mime_for_path(&hit.path).is_some_and(documents::is_playable);
                    let is_playing = self.player.is_playing() && self.player.current_path() == Some(&hit.path);
                    let hit_path_copy = hit.path.clone();

                    if is_media {
                        let play_label = if is_playing { "Pause" } else { "Play" };
                        let play_icon = if is_playing { IconName::Pause } else { IconName::Play };
                        action_btn = action_btn.item(
                            MacMenuItem::new("action-play", play_label)
                                .icon(play_icon)
                                .shortcut("Space")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.toggle_media_playback(cx);
                                })),
                        );
                    }

                    action_btn = action_btn
                        .item(
                            MacMenuItem::new("action-open", "Open in Default App")
                                .icon(IconName::ExternalLink)
                                .shortcut("↵")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.open_selected(false, cx);
                                })),
                        )
                        .item(
                            MacMenuItem::new("action-reveal", "Reveal in Finder")
                                .icon(IconName::Folder)
                                .shortcut("⌘↵")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.open_selected(true, cx);
                                })),
                        )
                        .item(
                            MacMenuItem::new("action-copy", "Copy File / Image")
                                .icon(IconName::Copy)
                                .shortcut("⌘C")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.copy_selected(cx);
                                })),
                        )
                        .item(
                            MacMenuItem::new("action-copy-path", "Copy Full Path")
                                .shortcut("⌥⌘C")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let path_str = hit_path_copy.display().to_string();
                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path_str.clone()));
                                    this.status = format!("Copied path: {}", path_str).into();
                                    cx.notify();
                                })),
                        );
                }

                toolbar = toolbar.trailing(action_btn);
            }

            if !count_label.is_empty() {
                toolbar = toolbar.trailing(MacBadge::new("count-badge", count_label));
            }

            toolbar.trailing(
                MacButton::new("settings-btn")
                    .borderless()
                    .icon(IconName::Settings)
                    .tooltip("Settings (⌘,)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_settings(&OpenSettings, window, cx);
                    })),
            )
        } else {
            toolbar = toolbar.leading(
                MacButton::new("settings-back-btn")
                    .borderless()
                    .icon(IconName::ChevronLeft)
                    .label("Search")
                    .tooltip("Back to Photos (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.present(window, cx);
                    })),
            );
            toolbar = toolbar.title("Settings");

            if !count_label.is_empty() {
                toolbar = toolbar.trailing(MacBadge::new("count-badge", count_label));
            }

            toolbar.trailing(
                MacButton::new("settings-done-btn")
                    .prominent()
                    .label("Done")
                    .tooltip("Close Settings (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.present(window, cx);
                    })),
            )
        }
    }

    fn render_index_banner(&self, cx: &Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let (note, loading, frac) = if self.fetching {
            (
                self.fetch_note.clone(),
                self.fetch_frac <= 1.,
                self.fetch_frac,
            )
        } else {
            (self.index_note.clone(), true, 0.)
        };
        let mut banner = div()
            .w_full()
            .h(px(INDEX_BANNER_HEIGHT))
            .px_5()
            .border_t_1()
            .border_color(border)
            .bg(cx.theme().primary.opacity(0.12))
            .flex()
            .items_center()
            .gap_3();

        if loading && frac <= 0.0 {
            banner = banner.child(MacSpinner::new("banner-spinner").size(MacSpinnerSize::Small));
        }

        banner
            .child(
                MacProgress::new("work-progress")
                    .value(frac)
                    .indeterminate(loading && frac <= 0.0)
                    .w(px(96.)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(note),
            )
    }

    fn work_height(&self, window: &Window) -> f32 {
        let h = f32::from(window.viewport_size().height);
        let mut chrome = TITLE_CHROME + STATUS_BAR_HEIGHT;
        if matches!(self.screen, Screen::Search) && !self.fetching {
            chrome += SEARCH_BAR_HEIGHT;
        }
        if self.busy || (self.fetching && matches!(self.screen, Screen::Settings)) {
            chrome += INDEX_BANNER_HEIGHT;
        }
        (h - chrome).max(160.)
    }

    fn settings_inner_width(window: &Window) -> f32 {
        let vp_w = f32::from(window.viewport_size().width);
        760.0f32.min(vp_w - 64.0).max(360.0)
    }

    fn first_launch_view(&self, work_h: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .id("first-launch")
            .w_full()
            .h(px(work_h))
            .child(
                EmptyState::new()
                    .h_full()
                    .w_full()
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(IconName::Sparkles),
                            )
                            .title(
                                EmptyTitle::new()
                                    .text_lg()
                                    .child("Downloading the on-device model"),
                            )
                            .description(
                                EmptyDescription::new().child(
                                    "About 530 MB, once. You can hide this window. Loupe stays in the menu bar — come back when the download finishes.",
                                ),
                            ),
                    )
                    .content(
                        EmptyContent::new()
                            .child(
                                v_flex()
                                    .gap_3()
                                    .w(px(360.))
                                    .items_center()
                                    .child(
                                        MacProgress::new("first-fetch")
                                            .value(self.fetch_frac)
                                            .indeterminate(self.fetch_frac <= 0.0)
                                            .w_full(),
                                    )
                                    .children((self.fetch_frac <= 0.0).then(|| {
                                        MacSpinner::new("first-fetch-spinner").size(MacSpinnerSize::Regular)
                                    }))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(muted)
                                            .child(self.fetch_note.clone()),
                                    )
                                    .child(
                                        MacButton::new("first-hide")
                                            .small()
                                            .borderless()
                                            .label("Hide to Menu Bar")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.hide_to_tray(cx);
                                            })),
                                    ),
                            ),
                    ),
            )
    }

    fn welcome_view(
        &self,
        work_h: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let total_photos: i64 = self
            .rows
            .iter()
            .filter_map(|r| match &r.index {
                IndexState::Ready { photos, .. } => Some(*photos),
                _ => None,
            })
            .sum();
        let muted = cx.theme().muted_foreground;
        let global = shortcut_label(&self.settings);
        let slides = SUGGESTED_QUERIES.iter().enumerate().map(|(index, &suggestion)| {
            CarouselItem::new(("welcome-slide", index), index, &self.welcome_carousel)
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    MacButton::new(SharedString::from(format!("welcome-{suggestion}")))
                        .borderless()
                        .label(suggestion)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_query(suggestion, window, cx);
                        })),
                )
        });
        let dots = SUGGESTED_QUERIES.iter().enumerate().map(|(index, _)| {
            CarouselPaginationItem::new(("welcome-dot", index), index, &self.welcome_carousel)
        });

        v_flex()
            .id("welcome")
            .track_scroll(&self.hits_scroll)
            .w_full()
            .h(px(work_h))
            .child(
                EmptyState::new()
                    .h_full()
                    .w_full()
                    .header(
                        EmptyHeader::new()
                            .title(
                                EmptyTitle::new()
                                    .text_xl()
                                    .child("Find a photograph"),
                            )
                            .description(EmptyDescription::new().child(if total_photos > 0 {
                                format!("{total_photos} photographs on disk")
                            } else {
                                "Open Settings and index a folder, then describe a photo.".into()
                            })),
                    )
                    .content(
                        EmptyContent::new()
                            .w_full()
                            .max_w(px(520.))
                            .gap_6()
                            .child(
                                Carousel::new("welcome-carousel", &self.welcome_carousel)
                                    .w_full()
                                    .accessibility_label("Try a search")
                                    .child(
                                        CarouselContent::new(&self.welcome_carousel)
                                            .w_full()
                                            .h(px(72.))
                                            .children(slides),
                                    )
                                    .child(CarouselPagination::new().children(dots)),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_2()
                                    .pt_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted)
                                            .child("Shortcuts"),
                                    )
                                    .child(welcome_shortcut(
                                        &FocusSearch,
                                        SEARCH,
                                        "secondary-f",
                                        "Focus the search field",
                                        window,
                                        muted,
                                    ))
                                    .child(welcome_shortcut(
                                        &ToggleGrid,
                                        SEARCH,
                                        "secondary-o",
                                        "Contact sheet or list",
                                        window,
                                        muted,
                                    ))
                                    .child(welcome_shortcut(
                                        &OpenSettings,
                                        SEARCH,
                                        "secondary-,",
                                        "Settings",
                                        window,
                                        muted,
                                    ))
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .child(key_chip(
                                                        &MoveLeft,
                                                        SEARCH,
                                                        "←",
                                                        window,
                                                    ))
                                                    .child(key_chip(
                                                        &MoveRight,
                                                        SEARCH,
                                                        "→",
                                                        window,
                                                    ))
                                                    .child(key_chip(
                                                        &MoveUp,
                                                        SEARCH,
                                                        "↑",
                                                        window,
                                                    ))
                                                    .child(key_chip(
                                                        &MoveDown,
                                                        SEARCH,
                                                        "↓",
                                                        window,
                                                    )),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child("Move through photos"),
                                            ),
                                    )
                                    .child(welcome_shortcut(
                                        &OpenResult,
                                        SEARCH,
                                        "enter",
                                        "Open the selected photo",
                                        window,
                                        muted,
                                    ))
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_3()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_medium()
                                                    .child(global),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted)
                                                    .child("Open Loupe from anywhere"),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    fn no_results_view(&self, work_h: f32, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("no-results")
            .w_full()
            .h(px(work_h))
            .child(
                EmptyState::new()
                    .h_full()
                    .w_full()
                    .header(
                        EmptyHeader::new()
                            .media(
                                EmptyMedia::new()
                                    .with_variant(EmptyMediaVariant::Icon)
                                    .child(IconName::Image),
                            )
                            .title(
                                EmptyTitle::new()
                                    .text_lg()
                                    .child("Nothing matched that."),
                            )
                            .description(EmptyDescription::new().child(
                                "Try fewer words, or open Settings and index the folder that holds the photos.",
                            )),
                    )
                    .content(
                        EmptyContent::new().child(
                            MacButton::new("clear-query-btn")
                                .small()
                                .borderless()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.clear_query(window, cx);
                                })),
                        ),
                    ),
            )
    }

    fn visible_search_notes(&self) -> Vec<String> {
        self.notes
            .iter()
            .filter(|note| self.hits.is_empty() || !note.contains("unplugged"))
            .cloned()
            .collect()
    }

    fn search_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let work_h = self.work_height(window);
        let has_hits = !self.hits.is_empty();
        let is_empty_query = self.query.trim().is_empty();

        if self.fetching {
            return v_flex()
                .id("search-page")
                .w_full()
                .bg(cx.theme().background)
                .child(self.first_launch_view(work_h, cx));
        }

        let search_bar = h_flex()
            .id("search-bar")
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .flex_shrink_0()
            .px(px(PAGE_INSET))
            .border_b_1()
            .border_color(border)
            .bg(cx.theme().background)
            .items_center()
            .child(
                div()
                    .flex_1()
                    .child(
                        MacSearchField::new("main-search", self.query_input.clone())
                            .shortcut_hint("⌘F"),
                    ),
            );

        let list_content: AnyElement = if is_empty_query && !has_hits {
            self.welcome_view(work_h, window, cx).into_any_element()
        } else if !is_empty_query && !has_hits && !self.busy {
            self.no_results_view(work_h, cx).into_any_element()
        } else {
            self.hits_sheet(cx, work_h).into_any_element()
        };

        v_flex()
            .id("search-page")
            .w_full()
            .bg(cx.theme().background)
            .children(self.index_problem.as_ref().map(|(_, sentence)| {
                Alert::error("index-problem", sentence.clone())
                    .small()
                    .mx_6()
                    .mt_3()
            }))
            .children(self.visible_search_notes().into_iter().enumerate().map(|(i, note)| {
                Alert::info(("search-note", i), note)
                    .xsmall()
                    .mx_6()
                    .mt_1()
            }))
            .children(self.errors.iter().enumerate().map(|(i, err)| {
                Alert::error(("search-error", i), err.clone())
                    .xsmall()
                    .mx_6()
                    .mt_1()
            }))
            .child(search_bar)
            .child(list_content)
    }

    fn hits_sheet(&mut self, cx: &mut Context<Self>, work_h: f32) -> impl IntoElement {
        let metrics = self.sheet_metrics();
        let cols = metrics.cols.max(1);
        let tile_h = metrics.tile_h;
        let hits_len = self.hits.len();
        let item_sizes = if self.grid {
            let rows = hits_len.div_ceil(cols);
            let row_h = px(tile_h + TILE_CAPTION + TILE_GAP);
            Rc::new(vec![size(px(1.), row_h); rows])
        } else {
            Rc::new(vec![size(px(1.), px(LIST_ROW_HEIGHT)); hits_len])
        };
        let entity = cx.entity();
        let list_id = if self.grid { "hits-grid" } else { "hits-rows" };
        let list = v_virtual_list(
            entity,
            list_id,
            item_sizes,
            move |this, range, _, cx| {
                if this.grid {
                    range
                        .map(|row| this.grid_row(row, cx))
                        .collect::<Vec<_>>()
                } else {
                    range
                        .map(|ix| this.list_row(ix, cx))
                        .collect::<Vec<_>>()
                }
            },
        )
        .track_scroll(&self.hits_scroll)
        .size_full();

        let sheet = v_flex()
            .id("hits")
            .track_focus(&self.results_focus)
            .key_context(RESULTS)
            .on_action(cx.listener(Self::on_preview))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_open_settings))
            .w_full()
            .h_full()
            .relative()
            .child(
                div()
                    .id("hits-body")
                    .absolute()
                    .inset_0()
                    .p_3()
                    .child(list),
            )
            .child(Scrollbar::vertical(&self.hits_scroll));

        div()
            .id("sheet")
            .w_full()
            .h(px(work_h))
            .child(
                h_resizable("sheet-split")
                    .size(px(work_h))
                    .child(resizable_panel().child(sheet))
                    .child(
                        resizable_panel()
                            .size(px(PREVIEW_WIDTH))
                            .size_range(px(280.)..px(720.))
                            .child(self.side_pane(cx)),
                    ),
            )
    }

    fn grid_row(&mut self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let metrics = self.sheet_metrics();
        let cols = metrics.cols.max(1);
        let start = row * cols;
        let selected = self.selected;
        let mut row_el = h_flex()
            .w_full()
            .gap(px(TILE_GAP))
            .items_start()
            .pb(px(TILE_GAP));
        for offset in 0..cols {
            let index = start + offset;
            if index >= self.hits.len() {
                break;
            }
            let hit = self.hits[index].clone();
            row_el = row_el.child(self.tile(
                index,
                hit,
                selected == index,
                metrics.tile_w,
                metrics.tile_h,
                cx,
            ));
        }
        row_el.into_any_element()
    }

    fn list_row(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(hit) = self.hits.get(index).cloned() else {
            return div().w_full().h(px(LIST_ROW_HEIGHT)).into_any_element();
        };
        let selected = self.selected == index;
        self.hit_row(index, hit, selected, cx).into_any_element()
    }

    fn side_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let mut pane = v_flex()
            .id("side-preview")
            .size_full()
            .min_h(px(0.))
            .gap_3()
            .border_l_1()
            .border_color(border)
            .bg(cx.theme().background)
            .p_4()
            .overflow_y_scrollbar();

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

        let mime = documents::mime_for_path(&hit.path);
        let is_audio = mime.is_some_and(documents::is_audio);
        let is_video = mime.is_some_and(documents::is_video);
        let is_playable = is_audio || is_video;

        let media_stage = if let Some(p) = preview_img {
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(photo_image(p, "side-preview-img", ObjectFit::Contain))
                .when(is_video, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_2()
                            .left_2()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded_full()
                            .bg(rgba(0x1C1C1ECC))
                            .border_1()
                            .border_color(rgba(0xFFFFFF33))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(svg().path(IconName::Video.path()).size(px(12.0)).text_color(rgb(0xFFFFFF)))
                            .child(div().mac_caption2().font_semibold().text_color(rgb(0xFFFFFF)).child("VIDEO"))
                    )
                })
                .when(is_audio, |el| {
                    el.child(
                        div()
                            .absolute()
                            .top_2()
                            .left_2()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded_full()
                            .bg(rgba(0x1C1C1ECC))
                            .border_1()
                            .border_color(rgba(0xFFFFFF33))
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(svg().path(IconName::Music.path()).size(px(12.0)).text_color(rgb(0xFFFFFF)))
                            .child(div().mac_caption2().font_semibold().text_color(rgb(0xFFFFFF)).child("AUDIO"))
                    )
                })
                .into_any_element()
        } else if is_audio {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .child(
                    div()
                        .w(px(72.0))
                        .h(px(72.0))
                        .rounded(px(16.0))
                        .bg(if cx.theme().mode.is_dark() { rgba(0xFFFFFF15) } else { rgba(0x0000000D) })
                        .border_1()
                        .border_color(if cx.theme().mode.is_dark() { rgba(0xFFFFFF20) } else { rgba(0x00000015) })
                        .shadow_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(IconName::Music.path())
                                .size(px(34.0))
                                .text_color(cx.theme().primary)
                        )
                )
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_center()
                        .child(name.clone())
                )
                .when_some(documents::audio_tag_info(&hit.path), |el, tags| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .text_center()
                            .child(tags)
                    )
                })
                .into_any_element()
        } else if is_video {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .w(px(64.0))
                        .h(px(64.0))
                        .rounded(px(12.0))
                        .bg(if cx.theme().mode.is_dark() { rgba(0xFFFFFF15) } else { rgba(0x0000000D) })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path(IconName::Video.path())
                                .size(px(32.0))
                                .text_color(cx.theme().primary)
                        )
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("Video Clip")
                )
                .into_any_element()
        } else {
            Skeleton::new().size_full().into_any_element()
        };

        pane = pane.child(
            div()
                .w_full()
                .flex_1()
                .min_h(px(320.))
                .overflow_hidden()
                .rounded(px(8.0))
                .bg(if cx.theme().mode.is_dark() { rgb(0x202023) } else { rgb(0xF2F2F7) })
                .flex()
                .items_center()
                .justify_center()
                .child(media_stage)
        );

        if is_playable {
            let is_current = self.player.current_path() == Some(&hit.path);
            let is_playing = is_current && self.player.is_playing();
            let cur_time = if is_current {
                self.player.current_time().unwrap_or(0.0)
            } else {
                0.0
            };
            let duration = if is_current {
                self.player.duration()
            } else {
                query_file_duration(&hit.path)
            };
            let dur = duration.unwrap_or(0.0);
            let fraction = if dur > 0.0 { (cur_time / dur).clamp(0.0, 1.0) } else { 0.0 };

            let elapsed_str = format_duration(cur_time);
            let remaining_str = if dur > cur_time {
                format!("-{}", format_duration(dur - cur_time))
            } else {
                format_duration(dur)
            };

            let hit_path_player = hit.path.clone();

            pane = pane.child(
                v_flex()
                    .id("in-app-player")
                    .w_full()
                    .p_3()
                    .rounded(px(10.0))
                    .bg(if cx.theme().mode.is_dark() { rgb(0x28282A) } else { rgb(0xEAEAEF) })
                    .border_1()
                    .border_color(if cx.theme().mode.is_dark() { rgba(0xFFFFFF15) } else { rgba(0x00000015) })
                    .gap_2()
                    .child(
                        div()
                            .id("player-scrubber-track")
                            .w_full()
                            .h(px(6.0))
                            .rounded_full()
                            .bg(if cx.theme().mode.is_dark() { rgba(0xFFFFFF22) } else { rgba(0x00000018) })
                            .relative()
                            .child(
                                div()
                                    .h_full()
                                    .w(gpui_kit::DefiniteLength::Fraction(fraction as f32))
                                    .rounded_full()
                                    .bg(cx.theme().primary)
                            )
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .mac_caption2()
                            .text_color(muted_fg)
                            .child(elapsed_str)
                            .child(remaining_str)
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child({
                                let hit_path_restart = hit_path_player.clone();
                                MacButton::new("player-restart")
                                    .small()
                                    .borderless()
                                    .tooltip("Restart (0:00)")
                                    .label("0:00")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.player.current_path() != Some(&hit_path_restart) {
                                            this.player.load(&hit_path_restart);
                                        }
                                        this.seek_media(0.0, cx);
                                    }))
                            })
                            .child({
                                let hit_path_back = hit_path_player.clone();
                                let target_seek = (cur_time - 15.0).max(0.0);
                                MacButton::new("player-back-15")
                                    .small()
                                    .borderless()
                                    .tooltip("Back 15s")
                                    .label("-15s")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.player.current_path() != Some(&hit_path_back) {
                                            this.player.load(&hit_path_back);
                                        }
                                        this.seek_media(target_seek, cx);
                                    }))
                            })
                            .child({
                                MacButton::new("player-play-toggle")
                                    .prominent()
                                    .icon(if is_playing { IconName::Pause } else { IconName::Play })
                                    .label(if is_playing { "Pause" } else { "Play" })
                                    .tooltip("Play / Pause (Space)")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_media_playback(cx);
                                    }))
                            })
                            .child({
                                let hit_path_fwd = hit_path_player.clone();
                                let target_seek = (cur_time + 15.0).min(dur);
                                MacButton::new("player-fwd-15")
                                    .small()
                                    .borderless()
                                    .tooltip("Forward 15s")
                                    .label("+15s")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if this.player.current_path() != Some(&hit_path_fwd) {
                                            this.player.load(&hit_path_fwd);
                                        }
                                        this.seek_media(target_seek, cx);
                                    }))
                            })
                    )
            );
        }

        let is_playing_this = self.player.is_playing() && self.player.current_path() == Some(&hit.path);

        pane = pane
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
            .child({
                let score_pct = (hit.similarity * 100.0).clamp(0.0, 100.0);
                let score_badge = MacBadge::new("side-score-badge", format!("{score_pct:.0}% match")).accent();
                let mut badge_row = h_flex().items_center().gap_2().flex_wrap().child(score_badge);
                if let Some(ext) = hit.path.extension().and_then(|e| e.to_str()) {
                    badge_row = badge_row.child(MacBadge::new("side-ext-badge", ext.to_uppercase()).neutral());
                }
                if is_audio {
                    badge_row = badge_row.child(MacBadge::new("side-media-badge", "AUDIO").neutral());
                } else if is_video {
                    badge_row = badge_row.child(MacBadge::new("side-media-badge", "VIDEO").neutral());
                }
                if is_playing_this {
                    badge_row = badge_row.child(MacBadge::new("side-playing-badge", "NOW PLAYING").accent());
                }
                if let Some(cam) = &hit.camera_model {
                    badge_row = badge_row.child(MacBadge::new("side-cam-badge", cam.clone()).neutral());
                }
                badge_row
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_wrap()
                    .w_full()
                    .child(
                        MacButton::new("side-open")
                            .small()
                            .prominent()
                            .icon(IconName::ExternalLink)
                            .label("Open")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.status = match open_path(&hit_path) {
                                    Ok(()) => format!("Opened {}", hit_path.display()).into(),
                                    Err(err) => err.to_string().into(),
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        MacButton::new("side-reveal")
                            .small()
                            .icon(IconName::Folder)
                            .label("Finder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.status = match reveal_path(&hit_reveal) {
                                    Ok(()) => "Showing it in Finder.".into(),
                                    Err(err) => err.to_string().into(),
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        MacButton::new("side-copy")
                            .small()
                            .icon(IconName::Copy)
                            .label("Copy")
                            .tooltip("Copy file to clipboard (⌘C)")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.copy_selected(cx);
                            })),
                    )
                    .child({
                        let hit_path_action = hit.path.clone();
                        let hit_path_copy = hit.path.clone();
                        let hit_reveal_action = hit.path.clone();
                        let is_media = documents::mime_for_path(&hit.path).is_some_and(documents::is_playable);
                        let is_playing = self.player.is_playing() && self.player.current_path() == Some(&hit.path);

                        let mut side_actions = MacPullDownButton::new("side-actions-menu", "Actions")
                            .icon(IconName::Sparkles)
                            .anchor(Anchor::TopRight);

                        if is_media {
                            let play_label = if is_playing { "Pause" } else { "Play" };
                            let play_icon = if is_playing { IconName::Pause } else { IconName::Play };
                            side_actions = side_actions.item(
                                MacMenuItem::new("side-act-play", play_label)
                                    .icon(play_icon)
                                    .shortcut("Space")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.toggle_media_playback(cx);
                                    })),
                            );
                        }

                        side_actions
                            .item(
                                MacMenuItem::new("side-act-open", "Open in Default App")
                                    .icon(IconName::ExternalLink)
                                    .shortcut("↵")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.status = match open_path(&hit_path_action) {
                                            Ok(()) => format!("Opened {}", hit_path_action.display()).into(),
                                            Err(err) => err.to_string().into(),
                                        };
                                        cx.notify();
                                    })),
                            )
                            .item(
                                MacMenuItem::new("side-act-reveal", "Reveal in Finder")
                                    .icon(IconName::Folder)
                                    .shortcut("⌘↵")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.status = match reveal_path(&hit_reveal_action) {
                                            Ok(()) => "Showing it in Finder.".into(),
                                            Err(err) => err.to_string().into(),
                                        };
                                        cx.notify();
                                    })),
                            )
                            .item(
                                MacMenuItem::new("side-act-copy", "Copy File / Image")
                                    .icon(IconName::Copy)
                                    .shortcut("⌘C")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.copy_selected(cx);
                                    })),
                            )
                            .item(
                                MacMenuItem::new("side-act-copy-path", "Copy Full Path")
                                    .shortcut("⌥⌘C")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        let path_str = hit_path_copy.display().to_string();
                                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(path_str.clone()));
                                        this.status = format!("Copied path: {}", path_str).into();
                                        cx.notify();
                                    })),
                            )
                    }),
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

        if is_playable {
            if let Some(dur) = query_file_duration(&hit.path) {
                facts = facts.item("Duration", format_duration(dur), 1);
            }
            if let Some(tag_info) = documents::audio_tag_info(&hit.path) {
                facts = facts.item("Audio Info", tag_info, 1);
            }
        } else {
            if let Some(camera) = &hit.camera_model {
                facts = facts.item("Camera", camera.clone(), 1);
            }
            if let Some(mm) = hit.focal_length_mm {
                facts = facts.item("Focal Length", format!("{mm:.0} mm"), 1);
            }
            if let Some(aperture) = hit.aperture_f {
                facts = facts.item("Aperture", format!("f/{aperture:.1}"), 1);
            }
        }

        let components: Vec<_> = hit.path.components().collect();
        let mut path_ctrl = MacPathControl::new("side-path-control");
        let comp_len = components.len();
        for (idx, comp) in components.iter().enumerate() {
            let comp_str = comp.as_os_str().to_string_lossy().to_string();
            if idx == 0 && comp_str == "/" {
                path_ctrl = path_ctrl.item_with_icon("Macintosh HD", IconName::HardDrive);
            } else if idx == comp_len - 1 {
                if is_audio {
                    path_ctrl = path_ctrl.item_with_icon(comp_str, IconName::Music);
                } else if is_video {
                    path_ctrl = path_ctrl.item_with_icon(comp_str, IconName::Video);
                } else {
                    path_ctrl = path_ctrl.item_with_icon(comp_str, IconName::Image);
                }
            } else {
                path_ctrl = path_ctrl.item_with_icon(comp_str, IconName::Folder);
            }
        }
        let hit_path_for_select = hit.path.clone();
        path_ctrl = path_ctrl.on_select(move |idx, _, _| {
            let subpath: PathBuf = hit_path_for_select.components().take(idx + 1).collect();
            let _ = reveal_path(&subpath);
        });

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
                        .child(div().text_xs().text_color(muted_fg).child("Location"))
                        .child(
                            MacButton::new("copy-path-btn")
                                .small()
                                .borderless()
                                .icon(IconName::Copy)
                                .tooltip("Copy path")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let path_str = hit_copy_path.display().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(path_str));
                                    this.status = "Copied file path to clipboard.".into();
                                    cx.notify();
                                })),
                        ),
                )
                .child(path_ctrl)
                .child(div().text_xs().text_color(muted_fg.opacity(0.7)).child(path)),
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

        let is_dark = cx.theme().mode.is_dark();
        let score_pct = (hit.similarity * 100.0).clamp(0.0, 100.0);
        let score_color = if score_pct >= 80.0 {
            rgb(0x34C759)
        } else if score_pct >= 60.0 {
            rgb(0x007AFF)
        } else {
            rgb(0xFF9500)
        };

        let ext_str = hit.path.extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_uppercase());

        let bg = if selected {
            if is_dark {
                rgba(0xFFFFFF1F)
            } else {
                rgba(0x007AFF14)
            }
        } else if is_dark {
            rgba(0xFFFFFF08)
        } else {
            rgba(0x00000004)
        };

        let border_color: Hsla = if selected {
            cx.theme().primary
        } else if is_dark {
            rgba(0xFFFFFF12).into()
        } else {
            rgba(0x0000000E).into()
        };

        let mime = documents::mime_for_path(&hit.path);
        let is_audio = mime.is_some_and(documents::is_audio);
        let is_video = mime.is_some_and(documents::is_video);
        let is_playing = self.player.is_playing() && self.player.current_path() == Some(&hit.path);

        let mut specs = Vec::new();
        specs.push(hit.label.clone());
        if is_audio {
            if let Some(tag) = documents::audio_tag_info(&hit.path) {
                specs.push(tag);
            }
            if let Some(dur) = query_file_duration(&hit.path) {
                specs.push(format_duration(dur));
            }
        } else if is_video {
            if let Some(dur) = query_file_duration(&hit.path) {
                specs.push(format_duration(dur));
            }
        } else {
            if let Some(camera) = &hit.camera_model {
                specs.push(camera.clone());
            }
            if let Some(focal) = hit.focal_length_mm {
                specs.push(format!("{focal:.0}mm"));
            }
            if let Some(aperture) = hit.aperture_f {
                specs.push(format!("f/{aperture:.1}"));
            }
        }
        let meta_line = specs.join("  •  ");

        let row_thumb: AnyElement = if let Some(p) = thumb.as_deref().filter(|p| p.is_file()) {
            thumb_box(Some(p), 96., 68., ("row-thumb", index), cx)
        } else if is_audio {
            div()
                .w(px(96.0))
                .h(px(68.0))
                .overflow_hidden()
                .rounded(px(6.0))
                .border_1()
                .border_color(if is_dark { rgba(0xFFFFFF1F) } else { rgba(0x00000014) })
                .bg(if is_dark { rgb(0x202023) } else { rgb(0xEAEAEF) })
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(IconName::Music.path())
                        .size(px(28.0))
                        .text_color(if is_dark { rgba(0xFFFFFF88) } else { rgba(0x00000066) })
                )
                .into_any_element()
        } else if is_video {
            div()
                .w(px(96.0))
                .h(px(68.0))
                .overflow_hidden()
                .rounded(px(6.0))
                .border_1()
                .border_color(if is_dark { rgba(0xFFFFFF1F) } else { rgba(0x00000014) })
                .bg(if is_dark { rgb(0x202023) } else { rgb(0xEAEAEF) })
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path(IconName::Video.path())
                        .size(px(28.0))
                        .text_color(if is_dark { rgba(0xFFFFFF88) } else { rgba(0x00000066) })
                )
                .into_any_element()
        } else {
            thumb_box(None, 96., 68., ("row-thumb", index), cx)
        };

        h_flex()
            .id(("hit", index))
            .w_full()
            .h(px(LIST_ROW_HEIGHT))
            .flex_shrink_0()
            .gap_3()
            .items_center()
            .px_3()
            .my(px(2.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(border_color)
            .bg(bg)
            .hover(|s| s.bg(if is_dark { rgba(0xFFFFFF14) } else { rgba(0x007AFF0A) }))
            .child(row_thumb)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(name),
                            )
                            .children(ext_str.map(|ext| {
                                div()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(px(3.0))
                                    .bg(if is_dark { rgba(0xFFFFFF1A) } else { rgba(0x0000000D) })
                                    .mac_caption2()
                                    .font_semibold()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(ext)
                            }))
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded_full()
                                    .bg(if is_dark { rgba(0xFFFFFF14) } else { rgba(0x0000000A) })
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded_full()
                                            .bg(score_color),
                                    )
                                    .child(
                                        div()
                                            .mac_caption2()
                                            .font_semibold()
                                            .text_color(if is_dark { rgb(0xFFFFFF) } else { rgb(0x1D1D1F) })
                                            .child(format!("{score_pct:.0}% match")),
                                    ),
                            )
                            .when(is_playing, |el| {
                                el.child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(1.0))
                                        .rounded_full()
                                        .bg(cx.theme().primary)
                                        .flex()
                                        .items_center()
                                        .gap(px(3.0))
                                        .child(
                                            svg()
                                                .path(IconName::Play.path())
                                                .size(px(8.0))
                                                .text_color(rgb(0xFFFFFF))
                                        )
                                        .child(
                                            div()
                                                .mac_caption2()
                                                .font_semibold()
                                                .text_color(rgb(0xFFFFFF))
                                                .child("PLAYING")
                                        )
                                )
                            }),
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
                        MacButton::new(SharedString::from(format!("open-row-{index}")))
                            .small()
                            .borderless()
                            .icon(IconName::ExternalLink)
                            .tooltip("Open photo (↵)")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = open_path(&path_clone);
                                this.status = format!("Opened {}", path_clone.display()).into();
                                cx.notify();
                            })),
                    )
                    .child(
                        MacButton::new(SharedString::from(format!("reveal-row-{index}")))
                            .small()
                            .borderless()
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
        tile_w: f32,
        tile_h: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = file_name(&hit);
        let thumb = self.shown_thumb(&hit);
        let label = hit.label.clone();
        let is_dark = cx.theme().mode.is_dark();
        let path_clone = hit.path.clone();
        let path_reveal = hit.path.clone();

        let score_pct = (hit.similarity * 100.0).clamp(0.0, 100.0);
        let score_color = if score_pct >= 80.0 {
            rgb(0x34C759)
        } else if score_pct >= 60.0 {
            rgb(0x007AFF)
        } else {
            rgb(0xFF9500)
        };

        let ext_str = hit.path.extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_uppercase());

        let mime = documents::mime_for_path(&hit.path);
        let is_audio = mime.is_some_and(documents::is_audio);
        let is_video = mime.is_some_and(documents::is_video);
        let is_playing = self.player.is_playing() && self.player.current_path() == Some(&hit.path);

        let mut camera_spec = String::new();
        if is_audio {
            if let Some(tag) = documents::audio_tag_info(&hit.path) {
                camera_spec = tag;
            } else if let Some(dur) = query_file_duration(&hit.path) {
                camera_spec = format_duration(dur);
            }
        } else if is_video {
            if let Some(dur) = query_file_duration(&hit.path) {
                camera_spec = format!("Video • {}", format_duration(dur));
            }
        } else {
            if let Some(focal) = hit.focal_length_mm {
                camera_spec.push_str(&format!("{focal:.0}mm"));
            }
            if let Some(aperture) = hit.aperture_f {
                if !camera_spec.is_empty() {
                    camera_spec.push_str(" • ");
                }
                camera_spec.push_str(&format!("f/{aperture:.1}"));
            }
            if let Some(cam) = &hit.camera_model {
                if !camera_spec.is_empty() {
                    camera_spec = format!("{cam} ({camera_spec})");
                } else {
                    camera_spec = cam.clone();
                }
            }
        }

        let card_bg = if selected {
            if is_dark {
                rgba(0xFFFFFF1F)
            } else {
                rgba(0x007AFF14)
            }
        } else if is_dark {
            rgba(0xFFFFFF0D)
        } else {
            rgba(0x00000006)
        };

        let card_border: Hsla = if selected {
            cx.theme().primary
        } else if is_dark {
            rgba(0xFFFFFF18).into()
        } else {
            rgba(0x00000012).into()
        };

        let mut img_container = div()
            .w_full()
            .h(px(tile_h))
            .rounded(px(6.0))
            .overflow_hidden()
            .relative()
            .bg(if is_dark { rgb(0x202023) } else { rgb(0xEAEAEF) })
            .child(
                if let Some(p) = thumb.as_deref().filter(|p| p.is_file()) {
                    photo_image(p, ("tile-img", index), ObjectFit::Cover)
                } else if is_audio {
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            svg()
                                .path(IconName::Music.path())
                                .size(px(36.0))
                                .text_color(if is_dark { rgba(0xFFFFFF88) } else { rgba(0x00000066) })
                        )
                        .into_any_element()
                } else if is_video {
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(
                            svg()
                                .path(IconName::Video.path())
                                .size(px(36.0))
                                .text_color(if is_dark { rgba(0xFFFFFF88) } else { rgba(0x00000066) })
                        )
                        .into_any_element()
                } else {
                    Skeleton::new().size_full().into_any_element()
                },
            );

        if is_playing {
            img_container = img_container.child(
                div()
                    .absolute()
                    .bottom_2()
                    .right_2()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded_full()
                    .bg(cx.theme().primary)
                    .shadow_sm()
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .child(
                        svg()
                            .path(IconName::Play.path())
                            .size(px(8.0))
                            .text_color(rgb(0xFFFFFF))
                    )
                    .child(
                        div()
                            .mac_caption2()
                            .font_semibold()
                            .text_color(rgb(0xFFFFFF))
                            .child("PLAYING")
                    ),
            );
        }

        // Top-right: Frosted glass match badge with confidence dot
        img_container = img_container.child(
            div()
                .absolute()
                .top_2()
                .right_2()
                .px(px(6.0))
                .py(px(2.0))
                .rounded_full()
                .bg(rgba(0x1C1C1ECC))
                .border_1()
                .border_color(rgba(0xFFFFFF33))
                .shadow_sm()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded_full()
                        .bg(score_color),
                )
                .child(
                    div()
                        .mac_caption2()
                        .font_semibold()
                        .text_color(rgb(0xFFFFFF))
                        .child(format!("{score_pct:.0}%")),
                ),
        );

        // Top-left: Format pill (e.g. HEIC, DNG, RAW)
        if let Some(ext) = ext_str {
            img_container = img_container.child(
                div()
                    .absolute()
                    .top_2()
                    .left_2()
                    .px(px(5.0))
                    .py(px(1.5))
                    .rounded(px(4.0))
                    .bg(rgba(0x1C1C1ECC))
                    .border_1()
                    .border_color(rgba(0xFFFFFF22))
                    .mac_caption2()
                    .font_semibold()
                    .text_color(rgba(0xFFFFFFD9))
                    .child(ext),
            );
        }

        // Bottom-left: External disk indicator if not This Mac
        if hit.label != "This Mac" {
            img_container = img_container.child(
                div()
                    .absolute()
                    .bottom_2()
                    .left_2()
                    .px(px(5.0))
                    .py(px(1.5))
                    .rounded(px(4.0))
                    .bg(rgba(0x1C1C1ECC))
                    .border_1()
                    .border_color(rgba(0xFFFFFF22))
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .child(
                        svg()
                            .path(IconName::HardDrive.path())
                            .size(px(9.0))
                            .text_color(rgba(0xFFFFFFD9)),
                    )
                    .child(
                        div()
                            .mac_caption2()
                            .text_color(rgba(0xFFFFFFD9))
                            .child(label.clone()),
                    ),
            );
        }

        // Bottom-right: Quick Action overlay when selected
        if selected {
            img_container = img_container.child(
                div()
                    .absolute()
                    .bottom_2()
                    .right_2()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        MacButton::new(SharedString::from(format!("tile-open-{index}")))
                            .small()
                            .borderless()
                            .icon(IconName::ExternalLink)
                            .tooltip("Open photo (↵)")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = open_path(&path_clone);
                                this.status = format!("Opened {}", path_clone.display()).into();
                                cx.notify();
                            })),
                    )
                    .child(
                        MacButton::new(SharedString::from(format!("tile-reveal-{index}")))
                            .small()
                            .borderless()
                            .icon(IconName::Folder)
                            .tooltip("Reveal in Finder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = reveal_path(&path_reveal);
                                this.status = "Showing in Finder.".into();
                                cx.notify();
                            })),
                    ),
            );
        }

        let mut card = div()
            .id(("tile", index))
            .w(px(tile_w))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .p(px(8.0))
            .bg(card_bg)
            .border(if selected { px(2.0) } else { px(1.0) })
            .border_color(card_border)
            .cursor_pointer();

        if selected {
            card = card.shadow_md();
        } else {
            card = card
                .shadow_sm()
                .hover(|s| s.bg(if is_dark { rgba(0xFFFFFF18) } else { rgba(0x0000000D) }).border_color(cx.theme().primary.opacity(0.4)).shadow_md());
        }

        card.child(img_container)
            .child(
                div()
                    .pt(px(6.0))
                    .px(px(2.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(if selected {
                                cx.theme().foreground
                            } else {
                                cx.theme().foreground.opacity(0.9)
                            })
                            .child(name),
                    )
                    .child(
                        div()
                            .mac_caption2()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(if !camera_spec.is_empty() {
                                camera_spec
                            } else {
                                label
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

    fn folders_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let disk_pick_root = disks.first().map(|(_, row)| row.path.clone());
        let inner_w = Self::settings_inner_width(window);

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

        let mut mac_list = v_flex().id("mac-folders").w(px(inner_w)).gap_2();
        for (index, row) in mac {
            mac_list = mac_list.child(self.folder_row(index, row, inner_w, cx));
        }

        let mut disk_list = v_flex().id("disk-folders").w(px(inner_w)).gap_2();
        for (index, row) in disks {
            disk_list = disk_list.child(self.folder_row(index, row, inner_w, cx));
        }

        let shortcut = shortcut_label(&self.settings);
        let problem = self.index_problem.as_ref().map(|(_, sentence)| sentence.clone());
        let muted = cx.theme().muted_foreground;

        let work_h = self.work_height(window);
        v_flex()
            .id("settings-page")
            .w(px(f32::from(window.viewport_size().width)))
            .h(px(work_h))
            .bg(cx.theme().background)
            .child(
                div()
                    .id("settings-scroll")
                    .w_full()
                    .h(px(work_h))
                    .overflow_y_scrollbar()
                    .flex()
                    .justify_center()
                    .child(
                        v_flex()
                            .id("settings-body")
                            .w(px(inner_w))
                            .gap_6()
                            .py_6()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .mac_large_title()
                                    .text_color(cx.theme().foreground)
                                    .child("Settings"),
                            )
                            .child(
                                div()
                                    .mac_subheadline()
                                    .text_color(muted)
                                    .child(summary),
                            ),
                    )
                    .children(problem.map(|sentence| {
                        Alert::error("folders-index-problem", sentence).small()
                    }))
                    .child(
                        v_flex()
                            .w(px(inner_w))
                            .gap_2()
                            .child(
                                h_flex()
                                    .w(px(inner_w))
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .text_sm()
                                            .font_semibold()
                                            .child(IconName::Folder)
                                            .child("This Mac"),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        MacButton::new("settings-add-folder")
                                            .small()
                                            .prominent()
                                            .icon(IconName::FolderPlus)
                                            .label("Add Folder")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.add_folder(cx);
                                            })),
                                    ),
                            )
                            .child(mac_list),
                    )
                    .child(
                        v_flex()
                            .w(px(inner_w))
                            .gap_2()
                            .child(
                                h_flex()
                                    .w(px(inner_w))
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .text_sm()
                                            .font_semibold()
                                            .child(IconName::HardDrive)
                                            .child("Plugged-in disks"),
                                    )
                                    .child(div().flex_1())
                                    .children((!no_disks).then(|| {
                                        let start = disk_pick_root.clone();
                                        MacButton::new("settings-disk-folder")
                                            .small()
                                            .prominent()
                                            .icon(IconName::FolderPlus)
                                            .label("Choose Folder")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                match start.clone() {
                                                    Some(path) => this.pick_folder_on_disk(path, cx),
                                                    None => this.add_folder(cx),
                                                }
                                            }))
                                    })),
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
                    .child(
                        v_flex()
                            .w(px(inner_w))
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
                            .child({
                                let (model_status_text, model_badge) = if self.fetching {
                                    (self.fetch_note.to_string(), MacBadge::new("download-badge", "Downloading").accent())
                                } else if gemma::weights_on_disk().is_some() {
                                    ("EmbeddingGemma 2 is ready on this Mac.".to_string(), MacBadge::new("installed-badge", "Installed").success())
                                } else {
                                    ("Model (~530 MB) downloads on first launch.".to_string(), MacBadge::new("not-installed-badge", "Available").neutral())
                                };

                                let mut model_group = MacSettingsGroup::new("engine-group")
                                    .title("Model")
                                    .footer("EmbeddingGemma 2 runs locally and privately on your Mac via Metal.")
                                    .row(
                                        MacSettingsRow::new("engine-row", "EmbeddingGemma 2")
                                            .subtitle(model_status_text)
                                            .icon(IconName::Cpu)
                                            .icon_color(cx.theme().primary)
                                            .control(model_badge),
                                    );

                                if self.fetching {
                                    model_group = model_group.child(
                                        v_flex()
                                            .w_full()
                                            .px(px(16.0))
                                            .pb(px(12.0))
                                            .gap_2()
                                            .child(
                                                MacProgress::new("settings-fetch")
                                                    .value(self.fetch_frac)
                                                    .indeterminate(self.fetch_frac <= 0.0)
                                                    .w_full(),
                                            ),
                                    );
                                }
                                model_group
                            })
                            .child(
                                MacSettingsGroup::new("theme-group")
                                    .title("Appearance")
                                    .footer("System appearance automatically follows macOS Light and Dark mode.")
                                    .row(
                                        MacSettingsRow::new("appearance-row", "Appearance")
                                            .subtitle("Follow macOS automatically, or choose Light or Dark")
                                            .icon(IconName::Sun)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                MacSegmentedControl::new("theme-mode")
                                                    .segment(MacSegment::new("system").label("System"))
                                                    .segment(MacSegment::new("light").label("Light"))
                                                    .segment(MacSegment::new("dark").label("Dark"))
                                                    .selected_index(match self.settings.theme.as_str() {
                                                        THEME_LIGHT | THEME_PAPER => 1,
                                                        THEME_DARK | THEME_DARKROOM => 2,
                                                        _ => 0,
                                                    })
                                                    .on_change(cx.listener(|this, idx: &usize, _, cx| {
                                                        let theme = match *idx {
                                                            1 => THEME_LIGHT,
                                                            2 => THEME_DARK,
                                                            _ => THEME_SYSTEM,
                                                        };
                                                        this.set_theme(theme, cx);
                                                    })),
                                            ),
                                    )
                                    .row({
                                        let accents = [
                                            ("Blue", "blue"),
                                            ("Purple", "purple"),
                                            ("Pink", "pink"),
                                            ("Red", "red"),
                                            ("Orange", "orange"),
                                            ("Yellow", "yellow"),
                                            ("Green", "green"),
                                            ("Graphite", "graphite"),
                                        ];
                                        let current_accent = self.settings.accent.to_ascii_lowercase();
                                        let selected_accent_idx = accents
                                            .iter()
                                            .position(|(_, id)| *id == current_accent)
                                            .unwrap_or(0);

                                        let mut accent_popup = MacPopupButton::new("accent-color-picker")
                                            .selected_index(selected_accent_idx);
                                        for (title, id) in &accents {
                                            accent_popup = accent_popup.item(MacMenuItem::new(*id, *title));
                                        }
                                        accent_popup = accent_popup.on_change(cx.listener(|this, idx: &usize, _, cx| {
                                            let accent_ids = ["blue", "purple", "pink", "red", "orange", "yellow", "green", "graphite"];
                                            if let Some(&name) = accent_ids.get(*idx) {
                                                this.set_accent(name, cx);
                                            }
                                        }));

                                        MacSettingsRow::new("accent-row", "Accent Color")
                                            .subtitle("System accent applied to controls and highlights")
                                            .icon(IconName::Sparkles)
                                            .icon_color(cx.theme().primary)
                                            .control(accent_popup)
                                    }),
                            )
                            .child(
                                MacSettingsGroup::new("menu-bar-group")
                                    .title("Menu Bar & Startup")
                                    .footer("Loupe lives in the macOS menu bar. Closing the window, Escape, or ⌘Q parks it.")
                                    .row(
                                        MacSettingsRow::new("open-at-login-row", "Open at Login")
                                            .subtitle("Start Loupe automatically when you log into your Mac")
                                            .icon(IconName::Power)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                MacSwitch::new("open-at-login")
                                                    .mini()
                                                    .checked(self.settings.open_at_login)
                                                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                                        this.set_open_at_login(*checked, cx);
                                                    })),
                                            ),
                                    ),
                            )
                            .child(
                                MacSettingsGroup::new("index-group")
                                    .title("Background Indexing")
                                    .footer("Sweep catches new copies and file changes that the watcher missed.")
                                    .row(
                                        MacSettingsRow::new("watch-folders-row", "Watch Folders")
                                            .subtitle("Index newly added photos while Loupe is in the background")
                                            .icon(IconName::Folder)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                MacSwitch::new("watch-folders")
                                                    .mini()
                                                    .checked(self.settings.watch_folders)
                                                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                                        this.set_watch_folders(*checked, cx);
                                                    })),
                                            ),
                                    )
                                    .row(
                                        MacSettingsRow::new("index-sweep-row", "Index Sweep Interval")
                                            .subtitle("Periodic background scan for disks and changed files")
                                            .icon(IconName::Clock)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                MacSegmentedControl::new("index-sweep")
                                                    .segment(MacSegment::new("off").label("Off"))
                                                    .segment(MacSegment::new("1h").label("1 hr"))
                                                    .segment(MacSegment::new("6h").label("6 hrs"))
                                                    .segment(MacSegment::new("daily").label("Daily"))
                                                    .selected_index(match self.settings.index_sweep_hours {
                                                        1 => 1,
                                                        6 => 2,
                                                        24 => 3,
                                                        _ => 0,
                                                    })
                                                    .on_change(cx.listener(|this, idx: &usize, _, cx| {
                                                        let hours = match *idx {
                                                            1 => 1,
                                                            2 => 6,
                                                            3 => 24,
                                                            _ => 0,
                                                        };
                                                        this.set_sweep_hours(hours, cx);
                                                    })),
                                            ),
                                    ),
                            )
                            .child({
                                let mut shortcut_group = MacSettingsGroup::new("shortcut-group")
                                    .title("Global Shortcut")
                                    .footer("Press modifier keys (Shift, Ctrl, Alt, Super) plus any key, then click Apply.")
                                    .row(
                                        MacSettingsRow::new("shortcut-row", "Open Search from Any App")
                                            .subtitle(format!("{shortcut} triggers Loupe from anywhere"))
                                            .icon(IconName::Keyboard)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(div().w(px(140.0)).child(Input::new(&self.shortcut_input)))
                                                    .child(
                                                        MacButton::new("apply-shortcut")
                                                            .small()
                                                            .label("Apply")
                                                            .on_click(cx.listener(|this, _, _, cx| this.apply_shortcut(cx))),
                                                    ),
                                            ),
                                    );
                                if !self.shortcut_error.is_empty() {
                                    shortcut_group = shortcut_group.child(
                                        div()
                                            .px(px(16.0))
                                            .pb(px(8.0))
                                            .text_xs()
                                            .text_color(cx.theme().danger)
                                            .child(self.shortcut_error.clone()),
                                    );
                                }
                                shortcut_group
                            })
                            .child(
                                MacSettingsGroup::new("update-group")
                                    .title("Software Updates")
                                    .footer(format!("Loupe v{APP_VERSION}"))
                                    .row(
                                        MacSettingsRow::new("daily-update-row", "Check Once a Day")
                                            .subtitle("Check GitHub releases daily for newer disk images")
                                            .icon(IconName::RefreshCw)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                MacSwitch::new("daily-update")
                                                    .mini()
                                                    .checked(self.settings.check_updates)
                                                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                                        this.settings.check_updates = *checked;
                                                        if let Err(err) = save_settings(&this.settings) {
                                                            this.update_line = err.to_string();
                                                        }
                                                        cx.notify();
                                                    })),
                                            ),
                                    )
                                    .row(
                                        MacSettingsRow::new("check-update-actions", "Update Status")
                                            .subtitle(if self.update_line.is_empty() {
                                                "Up to date".to_string()
                                            } else {
                                                self.update_line.clone()
                                            })
                                            .icon(IconName::Download)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(
                                                        MacButton::new("check-now")
                                                            .small()
                                                            .label("Check Now")
                                                            .on_click(cx.listener(|this, _, _, cx| this.spawn_update(cx))),
                                                    )
                                                    .children(self.release_url.clone().map(|url| {
                                                        MacButton::new("open-release")
                                                            .small()
                                                            .prominent()
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
                                    )
                                    .row(
                                        MacSettingsRow::new("feed-url-row", "Custom Feed URL")
                                            .subtitle("Optional custom release feed URL")
                                            .icon(IconName::Rss)
                                            .icon_color(cx.theme().primary)
                                            .control(
                                                h_flex()
                                                    .gap_2()
                                                    .items_center()
                                                    .child(div().w(px(200.0)).child(Input::new(&self.feed_input)))
                                                    .child(
                                                        MacButton::new("save-feed")
                                                            .small()
                                                            .label("Save")
                                                            .on_click(cx.listener(|this, _, _, cx| this.save_feed(cx))),
                                                    )
                                                    .child(
                                                        MacButton::new("clear-feed")
                                                            .small()
                                                            .borderless()
                                                            .label("Clear")
                                                            .on_click(cx.listener(|this, _, _, cx| this.clear_feed(cx))),
                                                    ),
                                            ),
                                    ),
                            ),
                    ),
                ),
            )
    }

    fn folder_row(
        &mut self,
        index: usize,
        row: FolderRow,
        row_w: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let path = row.path.clone();
        let index_path = path.clone();
        let disk_folder_path = path.clone();
        let forget_path = path.clone();
        let reveal_path_val = path.clone();
        let title = folder_title(&row);
        let hint = folder_hint(&row);
        let where_line = short_path(&path);
        let indexed = matches!(row.index, IndexState::Ready { .. });
        let connected = !matches!(row.index, IndexState::NotConnected);
        let remembered = row.remembered;
        let underway = self.indexing_path.as_ref() == Some(&path);
        let action = if underway {
            "Indexing…"
        } else if indexed {
            "Update"
        } else {
            "Index"
        };
        let muted = cx.theme().muted_foreground;
        let is_external = row.kind == DiskKind::External;
        let pip = if underway {
            cx.theme().warning
        } else {
            match &row.index {
                IndexState::Ready { .. } => cx.theme().primary,
                IndexState::Unreadable(_) => cx.theme().danger,
                _ => cx.theme().muted_foreground.opacity(0.35),
            }
        };

        let status_tag = if underway {
            MacBadge::new(SharedString::from(format!("tag-{index}")), "indexing").warning()
        } else {
            match &row.index {
                IndexState::Ready { photos, .. } => {
                    MacBadge::new(SharedString::from(format!("tag-{index}")), format!("{photos} photos")).neutral()
                }
                IndexState::NoIndex => MacBadge::new(SharedString::from(format!("tag-{index}")), "no index").neutral(),
                IndexState::NotConnected => MacBadge::new(SharedString::from(format!("tag-{index}")), "unplugged").neutral(),
                IndexState::Unreadable(_) => MacBadge::new(SharedString::from(format!("tag-{index}")), "unreadable").danger(),
            }
        };
        let live_line = underway.then(|| index_sentence(&title, &self.index_live));

        h_flex()
            .id(("folder", index))
            .w(px(row_w))
            .items_center()
            .gap_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.6))
            .child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .min_w(px(0.))
                    .child(
                        div()
                            .mt_1()
                            .w(px(8.))
                            .h(px(8.))
                            .rounded_full()
                            .bg(pip)
                            .flex_shrink_0(),
                    )
                    .child(
                        v_flex()
                            .gap_0p5()
                            .min_w(px(0.))
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_start()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_semibold()
                                            .text_sm()
                                            .flex_shrink_0()
                                            .child(title),
                                    )
                                    .child(status_tag)
                                    .children(is_external.then(|| {
                                        let medium_label = match row.medium {
                                            DiskMedium::Ssd => "SSD",
                                            DiskMedium::Hdd => "HDD",
                                            DiskMedium::Unknown => "DISK",
                                        };
                                        MacBadge::new(SharedString::from(format!("med-{index}")), medium_label).neutral()
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
                            )
                            .children(live_line.map(|line| {
                                v_flex()
                                    .gap_1()
                                    .pt_1()
                                    .child(
                                        MacProgress::new(("index-row", index))
                                            .indeterminate(true)
                                            .w(px(160.))
                                            .color(cx.theme().warning),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().warning)
                                            .child(line),
                                    )
                            })),
                    ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .flex_shrink_0()
                    .children(connected.then(|| {
                        MacButton::new(SharedString::from(format!("index-{index}")))
                            .small()
                            .variant(if underway || indexed {
                                MacButtonVariant::Push
                            } else {
                                MacButtonVariant::Prominent
                            })
                            .icon(IconName::RefreshCw)
                            .label(action)
                            .disabled(underway)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.start_index(index_path.clone(), cx);
                            }))
                    }))
                    .children((is_external && connected && !remembered).then(|| {
                        MacButton::new(SharedString::from(format!("disk-folder-{index}")))
                            .small()
                            .borderless()
                            .icon(IconName::FolderPlus)
                            .tooltip("Choose a folder on this disk")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pick_folder_on_disk(disk_folder_path.clone(), cx);
                            }))
                    }))
                    .children((!is_external && connected).then(|| {
                        MacButton::new(SharedString::from(format!("reveal-f-{index}")))
                            .small()
                            .borderless()
                            .icon(IconName::Folder)
                            .tooltip("Reveal in Finder")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let _ = reveal_path(&reveal_path_val);
                                this.status = "Showing in Finder.".into();
                                cx.notify();
                            }))
                    }))
                    .children(remembered.then(|| {
                        MacButton::new(SharedString::from(format!("forget-{index}")))
                            .small()
                            .borderless()
                            .destructive()
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
        let banner = (self.busy || (self.fetching && matches!(self.screen, Screen::Settings)))
            .then(|| self.render_index_banner(cx));
        let body: AnyElement = match self.screen {
            Screen::Settings => self.folders_page(window, cx).into_any_element(),
            Screen::Search => self.search_page(window, cx).into_any_element(),
        };
        let is_playable = self
            .hits
            .get(self.selected)
            .and_then(|h| documents::mime_for_path(&h.path))
            .is_some_and(documents::is_playable);
        let bottom_bar = search_hint(&self.settings, self.status.clone(), is_playable, window, cx);
        let viewport = window.viewport_size();

        v_flex()
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
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_open_settings))
            .w(viewport.width)
            .h(viewport.height)
            .bg(if cx.theme().mode.is_dark() {
                rgba(0x28282AEB)
            } else {
                rgba(0xF6F6F6EB)
            })
            .text_color(cx.theme().foreground)
            .child(top_bar)
            .child(body)
            .children(banner)
            .child(bottom_bar)
    }
}

impl Focusable for PhotoApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
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

fn folder_hint(row: &FolderRow) -> String {
    let line = match &row.index {
        IndexState::Ready { .. } if row.kind == DiskKind::External => "Search includes this disk.",
        IndexState::Ready { .. } if row.remembered => "Search includes this folder.",
        IndexState::Ready { .. } => "Included because an index is already here.",
        IndexState::NoIndex if row.kind == DiskKind::External => {
            "Index the whole disk, or choose a folder on it. The catalog is a hidden .photo_index folder."
        }
        IndexState::NoIndex if pictures_holds_photos_app(&row.path) => {
            "Index covers files sitting in this folder. The Photos app library is a package and is not scanned."
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

fn pictures_holds_photos_app(path: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| {
                let lower = name.to_ascii_lowercase();
                lower.ends_with(".photoslibrary")
                    || lower.ends_with(".photolibrary")
                    || lower.ends_with(".migratedphotolibrary")
            })
            .unwrap_or(false)
    })
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

fn photo_image(
    path: &std::path::Path,
    id: impl Into<ElementId>,
    fit: ObjectFit,
) -> AnyElement {
    img(Arc::<std::path::Path>::from(path.to_path_buf()))
        .id(id)
        .size_full()
        .object_fit(fit)
        .with_loading(|| Skeleton::new().size_full().into_any_element())
        .with_fallback(|| Skeleton::new().secondary().size_full().into_any_element())
        .into_any_element()
}

fn thumb_box(
    path: Option<&std::path::Path>,
    width: f32,
    height: f32,
    id: impl Into<ElementId>,
    cx: &App,
) -> AnyElement {
    let is_dark = cx.theme().mode.is_dark();
    let frame = div()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .rounded(px(6.))
        .border_1()
        .border_color(if is_dark { rgba(0xFFFFFF1F) } else { rgba(0x00000014) })
        .bg(if is_dark { rgb(0x202023) } else { rgb(0xEAEAEF) });
    if let Some(p) = path.filter(|p| p.is_file()) {
        frame
            .child(photo_image(p, id, ObjectFit::Cover))
            .into_any_element()
    } else {
        frame.child(Skeleton::new().size_full()).into_any_element()
    }
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
    is_playable: bool,
    window: &Window,
    _cx: &App,
) -> impl IntoElement {
    MacStatusBar::new("bottom-status-bar")
        .leading(
            div()
                .pl(px(PAGE_INSET))
                .child(status_text),
        )
        .trailing(
            h_flex()
                .items_center()
                .gap_3()
                .pr(px(PAGE_INSET))
                .when(is_playable, |el| {
                    el.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(key_chip(&RefreshPreview, RESULTS, "space", window))
                            .child("play / pause"),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&FocusSearch, SEARCH, "secondary-f", window))
                        .child("search"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(key_chip(&OpenSettings, SEARCH, "secondary-,", window))
                        .child("settings"),
                )
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
                        .child(key_chip(&Dismiss, SEARCH, "secondary-q", window))
                        .child("hide"),
                ),
        )
}

fn welcome_shortcut(
    action: &dyn Action,
    context: &str,
    fallback: &str,
    label: &str,
    window: &Window,
    muted: Hsla,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_3()
        .child(key_chip(action, context, fallback, window))
        .child(div().text_sm().text_color(muted).child(label.to_string()))
}

fn key_chip(action: &dyn Action, context: &str, fallback: &str, window: &Window) -> AnyElement {
    if let Some(kbd) = Kbd::binding_for_action(action, Some(context), window) {
        return kbd.outline().into_any_element();
    }
    let formatted = format_mac_shortcut(fallback);
    MacKbd::new(SharedString::from(format!("kchip-{fallback}")), formatted).into_any_element()
}

fn search_status(hits: &[RankedHit], notes: &[String], index_line: &str) -> SharedString {
    let unplugged = notes
        .iter()
        .find(|note| note.contains("unplugged"))
        .cloned()
        .or_else(|| {
            index_line
                .contains("unplugged")
                .then(|| index_line.to_string())
        });
    if hits.is_empty() {
        if let Some(note) = unplugged {
            if note.contains("still") {
                return note.into();
            }
            return format!("{note} Other folders still search.").into();
        }
        return "No matching photos.".into();
    }
    match unplugged {
        Some(note) => {
            let short = note
                .split(". ")
                .next()
                .unwrap_or(note.as_str())
                .trim_end_matches('.');
            format!("{} photos. {short}.", hits.len()).into()
        }
        None => format!("{} photos.", hits.len()).into(),
    }
}

struct SheetMetrics {
    cols: usize,
    tile_w: f32,
    tile_h: f32,
}

impl SheetMetrics {
    fn from_width(width: f32) -> Self {
        let inner = if width < SHEET_MIN_LAYOUT {
            SHEET_FALLBACK_INNER
        } else {
            (width - SHEET_PAD).max(SHEET_MIN_INNER)
        };
        let ideal_tile_w: f32 = 240.0;
        let cols = ((inner + TILE_GAP) / (ideal_tile_w + TILE_GAP))
            .floor()
            .clamp(1.0, 6.0) as usize;
        let tile_w = ((inner - TILE_GAP * (cols as f32 - 1.)) / cols as f32).floor();
        let tile_h = (tile_w * 0.72).floor().clamp(150., 320.);
        Self {
            cols,
            tile_w,
            tile_h,
        }
    }
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
    let spec = hotkey_spec(chord);
    HotKey::try_from(spec).map_err(|err| anyhow!("That shortcut is not available. {err}"))
}

fn hotkey_spec(chord: &Chord) -> String {
    let mut parts = Vec::new();
    if chord.shift {
        parts.push("SHIFT".to_string());
    }
    if chord.ctrl {
        parts.push("CONTROL".to_string());
    }
    if chord.alt {
        parts.push("ALT".to_string());
    }
    if chord.super_key {
        parts.push("SUPER".to_string());
    }
    let key = match &chord.key {
        ChordKey::Space => "SPACE".to_string(),
        ChordKey::Letter(ch) => ch.to_ascii_uppercase().to_string(),
        ChordKey::Digit(ch) => ch.to_string(),
        ChordKey::Function(n) => format!("F{n}"),
        ChordKey::Other(name) => name.to_uppercase(),
    };
    parts.push(key);
    parts.join("+")
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
        let img_path = std::env::temp_dir().join("loupe_test_copy.png");
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

    #[core::prelude::v1::test]
    fn test_dropdown_controls_construction() {
        let sort_btn = MacPopupButton::new("sort-order-btn")
            .item(MacMenuItem::new("sort-best", "Best Match").icon(IconName::Sparkles))
            .item(MacMenuItem::new("sort-az", "Name (A → Z)"))
            .item(MacMenuItem::new("sort-za", "Name (Z → A)"))
            .selected_index(1)
            .anchor(Anchor::TopRight);
        let _ = sort_btn;

        let action_btn = MacPullDownButton::new("toolbar-actions", "Action")
            .icon(IconName::Sparkles)
            .anchor(Anchor::TopRight)
            .item(MacMenuItem::new("act-open", "Open in Default App").shortcut("↵"))
            .item(MacMenuItem::new("act-reveal", "Reveal in Finder").shortcut("⌘↵"))
            .item(MacMenuItem::new("act-copy", "Copy File / Image").shortcut("⌘C"))
            .item(MacMenuItem::new("act-copy-path", "Copy Full Path").shortcut("⌥⌘C"));
        let _ = action_btn;
    }
}
