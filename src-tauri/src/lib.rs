// Thin IPC bindings only - every command delegates to the `knowdesk-core` crate
// (`CLAUDE.md`: "core는 Tauri를 절대 참조하지 않는다. 모든 OS 통합은 src-tauri로 격리한다").
// `open_path`/`open_parent_folder` are the exception (native opener, no equivalent in `core`).

use knowdesk_core::config::{Config, Theme};
use knowdesk_core::db::documents::DocumentRepository;
use knowdesk_core::db::Db;
use knowdesk_core::extract::ooxml::{DocxExtractor, PptxExtractor};
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::xlsx::XlsxExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::index::queue;
use knowdesk_core::index::watcher::FileWatcher;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::nlp::Tokenizer;
use knowdesk_core::scan::walker;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{
    MatchKind, SearchHit, SearchMode as CoreSearchMode, SearchRequest,
    SearchService as SearchServiceTrait,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

/// A search request sent to the dedicated worker thread (see `SearchWorker`).
struct SearchJob {
    query: String,
    mode: CoreSearchMode,
    limit: i64,
    reply: mpsc::Sender<Result<Vec<SearchHitDto>, String>>,
}

/// Confines the DB connection and the optional Kiwi tokenizer to one dedicated
/// thread and talks to it over a channel, rather than sharing them behind a
/// `Mutex` in Tauri's managed state. `kiwi_rs::Kiwi` isn't `Send` (its internal
/// caches hold `Box<dyn Fn>` rule callbacks), so it can never be moved into a
/// `Mutex<T>` that Tauri's command dispatch requires to be `Send + Sync` - it
/// has to stay on the single thread that created it for its entire lifetime.
struct SearchWorker {
    sender: mpsc::Sender<SearchJob>,
}

impl SearchWorker {
    fn spawn(db_path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel::<SearchJob>();
        thread::spawn(move || {
            let db = Db::open(&db_path).expect("failed to open index DB");
            let kiwi = load_kiwi();
            for job in receiver {
                let service = SqliteSearchService {
                    conn: &db.conn,
                    kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
                };
                let result = service
                    .search(&SearchRequest {
                        query: job.query,
                        mode: job.mode,
                        limit: job.limit,
                    })
                    .map(|r| r.hits.into_iter().map(SearchHitDto::from).collect())
                    .map_err(|e| e.to_string());
                let _ = job.reply.send(result);
            }
        });
        Self { sender }
    }

    fn search(
        &self,
        query: String,
        mode: CoreSearchMode,
        limit: i64,
    ) -> Result<Vec<SearchHitDto>, String> {
        let (reply, reply_rx) = mpsc::channel();
        self.sender
            .send(SearchJob {
                query,
                mode,
                limit,
                reply,
            })
            .map_err(|_| "search worker unavailable".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "search worker unavailable".to_string())?
    }
}

/// `KNOWDESK_DB_PATH` overrides the DB location - lets the same manual-testing
/// workflow used for `knowdesk-cli` (`--db ./samples.db`) point this app at an
/// already-indexed DB. Without it, defaults to a per-OS app-data directory
/// (`DEVELOPMENT.md`'s `--db`-less CLI behavior is cwd-relative, which isn't
/// appropriate for a GUI app launched from anywhere).
fn db_path() -> PathBuf {
    if let Ok(path) = std::env::var("KNOWDESK_DB_PATH") {
        return PathBuf::from(path);
    }
    app_data_dir().join("knowdesk.db")
}

/// Per-OS app-data directory shared by `db_path()` and `settings_path()` - both are
/// inherently machine-local (the DB is large and rebuildable, and `watched_folders`
/// holds absolute paths that only make sense on this machine), so neither belongs in
/// a roaming/synced profile location even where the OS distinguishes one.
fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("KnowDesk")
}

/// `KNOWDESK_SETTINGS_PATH` overrides the settings file location, same convention as
/// `KNOWDESK_DB_PATH` (`db_path()`). There's no Settings Window (TASK-704 was
/// replaced with `open_settings_file`, which just opens this file itself) -
/// `settings.json` is a plain text file the user edits by hand.
fn settings_path() -> PathBuf {
    if let Ok(path) = std::env::var("KNOWDESK_SETTINGS_PATH") {
        return PathBuf::from(path);
    }
    app_data_dir().join("settings.json")
}

/// Initializes Kiwi from `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`, same
/// as `knowdesk-cli`'s `load_kiwi()` - bigram is always the default tokenizer, so
/// there's nothing to fall back to; Kiwi is just `None` when not configured.
fn load_kiwi() -> Option<KiwiTokenizer> {
    let kiwi = match KiwiTokenizer::from_env() {
        Some(Ok(kiwi)) => Some(kiwi),
        Some(Err(e)) => {
            eprintln!("Kiwi initialization failed, using bigram only: {e}");
            None
        }
        None => None,
    };
    if kiwi.is_none() {
        eprintln!(
            "Notice: Kiwi morphological analyzer is not available — using bigram tokenization only."
        );
    }
    kiwi
}

/// Points `KNOWDESK_PDFIUM_LIB_DIR`/`KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`
/// at the native libraries/model bundled into the packaged `.app`
/// (`Contents/Resources/native/...` - see `tauri.conf.json`'s `bundle.resources`
/// and `docs/03_Architecture.md`). Called once at the very start of `run()`,
/// before `load_kiwi()`/the first PDF extraction can run.
///
/// An already-set env var always wins (local dev pointing at `.pdfium`/`.kiwi`
/// via the shell), and this is a no-op if the bundled files aren't actually
/// there - e.g. `cargo run`/`tauri dev` has no `Resources` directory next to
/// the dev binary. Either way, `core::extract::pdf`/`core::nlp::kiwi` already
/// fall back gracefully (PDF -> META tier, Korean search -> bigram only) when
/// these are unset, so a miss here never breaks anything - it only means the
/// packaged app runs in that reduced mode instead of fully offline-capable.
#[cfg(target_os = "macos")]
fn set_bundled_native_lib_env_vars() {
    let Some(resources) = std::env::current_exe().ok().and_then(|exe| {
        // Packaged layout: `KnowDesk.app/Contents/MacOS/<exe>` ->
        // `KnowDesk.app/Contents/Resources` (same convention Tauri's own
        // `resource_dir()` uses, replicated here since this runs before an
        // `AppHandle` exists).
        exe.parent().map(|dir| dir.join("../Resources/native"))
    }) else {
        return;
    };

    if std::env::var_os("KNOWDESK_PDFIUM_LIB_DIR").is_none() {
        let dir = resources.join("pdfium");
        if dir.join("libpdfium.dylib").is_file() {
            // SAFETY: called once, single-threaded, before any thread that
            // could read the environment concurrently is spawned.
            unsafe { std::env::set_var("KNOWDESK_PDFIUM_LIB_DIR", dir) };
        }
    }

    if std::env::var_os("KNOWDESK_KIWI_LIB_PATH").is_none()
        && std::env::var_os("KNOWDESK_KIWI_MODEL_DIR").is_none()
    {
        let lib_path = resources.join("kiwi/libkiwi.dylib");
        let model_dir = resources.join("kiwi/models");
        if lib_path.is_file() && model_dir.is_dir() {
            // SAFETY: see above.
            unsafe {
                std::env::set_var("KNOWDESK_KIWI_LIB_PATH", lib_path);
                std::env::set_var("KNOWDESK_KIWI_MODEL_DIR", model_dir);
            }
        }
    }
}

/// Matches `frontend/src/types.ts`'s `SearchHit`.
#[derive(serde::Serialize)]
struct SearchHitDto {
    path: String,
    filename: String,
    snippet: Option<String>,
    #[serde(rename = "matchKind")]
    match_kind: String,
    extension: String,
    #[serde(rename = "modifiedAt")]
    modified_at: Option<String>,
    #[serde(rename = "indexTier")]
    index_tier: String,
}

impl From<SearchHit> for SearchHitDto {
    fn from(hit: SearchHit) -> Self {
        Self {
            path: hit.path,
            filename: hit.filename,
            snippet: hit.snippet,
            match_kind: match hit.match_kind {
                MatchKind::Exact => "exact".to_string(),
                MatchKind::Morphological => "morphological".to_string(),
            },
            extension: hit.extension,
            modified_at: hit.modified_at,
            index_tier: hit.index_tier,
        }
    }
}

#[tauri::command]
fn search(
    worker: tauri::State<SearchWorker>,
    query: String,
    mode: String,
    limit: i64,
) -> Result<Vec<SearchHitDto>, String> {
    let search_mode = match mode.as_str() {
        "filename" => CoreSearchMode::Filename,
        _ => CoreSearchMode::Content,
    };
    worker.search(query, search_mode, limit)
}

/// `Enter` action (`docs/12_UI_Spec.md` C1): opens the file with the OS default
/// program.
#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<String>)
        .map_err(|e| e.to_string())
}

/// `Ctrl+Enter` action (`docs/12_UI_Spec.md` C1/C3): opens the containing folder.
#[tauri::command]
fn open_parent_folder(app: AppHandle, path: String) -> Result<(), String> {
    let parent = Path::new(&path)
        .parent()
        .ok_or_else(|| "No parent folder".to_string())?;
    app.opener()
        .open_path(parent.to_string_lossy().into_owned(), None::<String>)
        .map_err(|e| e.to_string())
}

/// "Settings" (tray menu): there's no in-app Settings window (TASK-704) -
/// opens `settings.json` itself with the OS default program for that file
/// type (a text editor, on every desktop OS this ships on) instead, so
/// editing that file directly is the whole UI. `run()` makes sure the file
/// actually exists with defaults before this could ever be called, so
/// there's always something real to open here.
#[tauri::command]
fn open_settings_file(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_path(
            settings_path().to_string_lossy().into_owned(),
            None::<String>,
        )
        .map_err(|e| e.to_string())
}

/// Reads the current `theme` setting from `settings.json` - called once at
/// page load and again every time the search window regains focus (shown via
/// the tray/hotkey), which is how a hand-edited theme setting takes effect
/// without needing to push a live-update event into an already-open webview.
#[tauri::command]
fn get_theme() -> Result<Theme, String> {
    Config::load(Some(&settings_path()))
        .map(|config| config.theme)
        .map_err(|e| e.to_string())
}

/// Reads the current `result_limit` setting - same "read on load + window
/// focus" pattern as `get_theme` above (see its doc comment): a search-result
/// count only needs to apply to the *next* search issued, so there's no need
/// for a live-push mechanism either.
#[tauri::command]
fn get_result_limit() -> Result<u32, String> {
    Config::load(Some(&settings_path()))
        .map(|config| config.result_limit)
        .map_err(|e| e.to_string())
}

/// Reads the current `search_debounce_ms` setting - same "read on load +
/// window focus" pattern as `get_theme`/`get_result_limit` above.
#[tauri::command]
fn get_search_debounce_ms() -> Result<u32, String> {
    Config::load(Some(&settings_path()))
        .map(|config| config.search_debounce_ms)
        .map_err(|e| e.to_string())
}

/// A hit's `snippet` is `null` when there's no keyword to build one around -
/// a filter-only query (e.g. `x:pdf`), or filename mode (never has one at
/// all). The frontend calls this on demand, only for the hit currently shown
/// in the preview pane, to fill in the document's opening text instead
/// (`docs/12_UI_Spec.md` C2) - not for every row in the result list, which
/// has no use for it (`frontend/src/components/kd-result-list.ts`).
const BODY_PREVIEW_CHARS: usize = 300;

#[tauri::command]
fn preview_body(path: String) -> Result<Option<String>, String> {
    let db = Db::open(&db_path()).map_err(|e| e.to_string())?;
    DocumentRepository::body_preview(&db.conn, &path, BODY_PREVIEW_CHARS).map_err(|e| e.to_string())
}

/// "색인 중 (done/total)" (TASK-904) - `None` while idle. Polled by the
/// search window, not pushed, same reasoning as `get_theme`'s "read on load +
/// focus" pattern - except this one keeps polling on an interval too while a
/// scan is actually in progress (`main.ts`), since unlike a settings value it
/// can change every moment the window is sitting open.
#[tauri::command]
fn get_index_progress(progress: tauri::State<IndexProgressState>) -> Option<IndexProgress> {
    *progress.lock().unwrap()
}

/// Shows the "search" window (pre-created hidden at startup, `tauri.conf.json`'s
/// `visible: false`) and gives it keyboard focus, or hides it again if it's
/// already visible (Spotlight/PowerToys Run convention -
/// `docs/12_UI_Spec.md` C4: "이미 열려 있는 검색창에서 단축키를 다시 누르면 →
/// 토글(다시 누르면 닫힘)"). Shared by the tray icon's left click and the
/// global hotkey.
/// Three states, not a plain visible/hidden toggle: visible *and* focused ->
/// hide; visible but not focused (the user clicked away to another app
/// without closing it) -> just refocus, without hiding first; hidden ->
/// show and focus. This way pressing the hotkey while it's already open but
/// not the active window brings it to the front instead of closing it.
fn toggle_search_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("search") {
        let visible = window.is_visible().unwrap_or(false);
        let focused = window.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = window.hide();
        } else if visible {
            let _ = window.set_focus();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// Shows and focuses the search window unconditionally, never hiding it -
/// used when a second launch attempt is detected
/// (`tauri_plugin_single_instance`, registered in `run()`), where the intent
/// is always "bring the app to the front", unlike the tray click/hotkey's
/// toggle behavior.
fn show_search_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("search") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Registers `hotkey` (`tauri-plugin-global-shortcut` string syntax, e.g.
/// `"CmdOrCtrl+Alt+K"`) to toggle the search window - shared by `run()`'s
/// initial registration and by live-reload when `settings.json`'s `hotkey`
/// field changes (see the `on_settings_reload` closure passed to
/// `spawn_index_worker` below). Registering an already-registered shortcut,
/// or one that fails to parse, returns an `Err` for the caller to log -
/// neither case should crash the app.
fn register_hotkey(
    app: &AppHandle,
    hotkey: &str,
) -> Result<(), tauri_plugin_global_shortcut::Error> {
    app.global_shortcut()
        .on_shortcut(hotkey, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                toggle_search_window(app);
            }
        })
}

/// Registers/unregisters the OS login item (`tauri-plugin-autostart`) to
/// match `auto_start` - shared by `run()`'s initial sync and by live-reload
/// when `settings.json`'s `auto_start` field changes (see the
/// `on_settings_reload` closure passed to `spawn_index_worker` below), same
/// pattern as `register_hotkey`. Best-effort - a failure here (e.g. the OS
/// denies the registration) is logged, not propagated, since it shouldn't
/// block the rest of startup or a reload.
fn sync_autostart(app: &AppHandle, enabled: bool) {
    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    if let Err(e) = result {
        let action = if enabled { "enable" } else { "disable" };
        eprintln!("Failed to {action} autostart: {e}");
    }
}

/// "Statistics" (tray menu, TASK-901): a human-readable index summary - total
/// documents, FULL/META breakdown, DB file size, last indexed time. `SKIP`
/// isn't included - unlike FULL/META, a skipped file never gets a
/// `documents` row at all (`core::index::pipeline`'s `index_file`), so
/// there's no persisted count to show for it. Opens its own short-lived DB
/// connection (like `knowdesk-cli stats`) - a plain read needs no
/// coordination with the index worker thread's own connection.
fn compute_stats(db_path: &Path) -> Result<String, String> {
    let db = Db::open(db_path).map_err(|e| e.to_string())?;
    let tiers = DocumentRepository::count_by_tier(&db.conn).map_err(|e| e.to_string())?;
    let count_of = |tier: &str| tiers.iter().find(|(t, _)| t == tier).map_or(0, |(_, c)| *c);
    let full = count_of("FULL");
    let meta = count_of("META");
    let last_indexed = DocumentRepository::last_indexed_at(&db.conn).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

    Ok(format!(
        "Total: {} documents\n  Full text indexed: {full}\n  Metadata only: {meta}\nDatabase size: {}\nLast indexed: {}",
        full + meta,
        format_bytes(size),
        last_indexed.as_deref().unwrap_or("never"),
    ))
}

/// Same formatting as `knowdesk-cli`'s own `format_bytes` (`cli/src/main.rs`) -
/// not shared as a dependency since it's a few lines and `cli` isn't a
/// library `src-tauri` depends on.
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1}{}", UNITS[unit])
}

fn default_extractors() -> Vec<Box<dyn ContentExtractor>> {
    vec![
        Box::new(TxtExtractor),
        Box::new(XlsxExtractor),
        Box::new(DocxExtractor),
        Box::new(PptxExtractor),
        Box::new(PdfExtractor),
    ]
}

/// Spawns the index worker thread. Always spawns, even with an empty initial
/// folder list (the common case until folders are added to `settings.json`),
/// since the thread just idles otherwise, ready for folders to show up via a
/// later hand-edit of the settings file (see `run_index_worker`).
///
/// `on_settings_reload` is called (with the just-replaced config and the one
/// now in effect) every time `settings.json` is successfully reloaded - `run()`
/// passes a closure that re-registers the global hotkey when it changes. Kept
/// as an injected callback rather than reaching for `AppHandle` directly in
/// `run_index_worker`, so that function (and its test,
/// `index_worker_applies_settings_file_changes_live`) stays free of any Tauri
/// runtime dependency - no mock app needed to call it.
fn spawn_index_worker(
    db_path: PathBuf,
    settings_path: PathBuf,
    config: Config,
    reset_rx: mpsc::Receiver<()>,
    on_settings_reload: impl Fn(&Config, &Config) + Send + 'static,
    progress: IndexProgressState,
) {
    thread::spawn(move || {
        if let Err(e) = run_index_worker(
            db_path,
            settings_path,
            config,
            reset_rx,
            on_settings_reload,
            progress,
        ) {
            eprintln!("Index worker failed: {e}");
        }
    });
}

/// Re-reads `settings.json` and returns the config to apply. Missing file
/// (deleted by hand) is recreated with defaults first, same as a first run
/// (`run()`). A parse error keeps `fallback` (the currently-applied config)
/// rather than falling back to defaults outright - a typo in a hand-edited
/// file shouldn't silently drop every already-configured folder.
fn reload_settings(settings_path: &Path, fallback: &Config) -> Config {
    if !settings_path.exists() {
        eprintln!("settings.json was deleted - recreating with defaults");
        let default = Config::default();
        if let Err(e) = default.save(settings_path) {
            eprintln!("Failed to recreate default settings.json: {e}");
        }
        return default;
    }
    Config::load(Some(settings_path)).unwrap_or_else(|e| {
        eprintln!("Failed to parse settings.json, keeping current settings: {e}");
        fallback.clone()
    })
}

/// Keeps every folder in `config.watched_folders` indexed for the rest of the
/// process's lifetime, and applies hand-edits to `settings.json` itself
/// automatically - no separate "reload"/"apply" action needed, since file
/// watching already exists for indexed folders and the settings file is just
/// one more file to apply the same idea to. Also watches `reset_rx` for a
/// "Reset Index" trigger from the tray menu.
fn run_index_worker(
    db_path: PathBuf,
    settings_path: PathBuf,
    initial_config: Config,
    reset_rx: mpsc::Receiver<()>,
    on_settings_reload: impl Fn(&Config, &Config),
    progress: IndexProgressState,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = Db::open(&db_path)?;
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let mut kiwi: Option<KiwiTokenizer> = None;
    let mut watched: Vec<PathBuf> = Vec::new();
    let mut config = initial_config;
    // Starts with no roots - `apply_folder_diff` below adds whatever
    // `initial_config` lists.
    let mut folder_watcher = FileWatcher::new::<PathBuf>(
        &[],
        Duration::from_millis(config.file_watch_debounce_ms.into()),
    )?;
    apply_folder_diff(
        &db,
        &extractors,
        &bigram,
        &mut kiwi,
        &mut folder_watcher,
        &mut watched,
        &config,
        &progress,
    );

    // Watches `settings.json`'s own folder (not the file directly - `notify`
    // watches on some platforms don't survive the file being deleted and
    // recreated, which is exactly the "삭제 시 기본값으로 재생성" case this
    // needs to catch) so a hand-edit or delete applies on its own. Fixed
    // internal value, not a `Config` field like `file_watch_debounce_ms` -
    // this is internal live-reload plumbing, not a user-facing concern.
    // Short (200ms, vs. the indexed-folder watcher's much longer default)
    // since a hand-edited settings file has no equivalent to the
    // "still-saving/temp-file" concern that debounce protects against there.
    const SETTINGS_WATCH_DEBOUNCE_MS: u64 = 200;
    let settings_filename = settings_path
        .file_name()
        .ok_or("settings path has no file name")?
        .to_owned();
    let settings_dir = settings_path
        .parent()
        .ok_or("settings path has no parent directory")?;
    let settings_watcher = FileWatcher::new(
        &[settings_dir],
        Duration::from_millis(SETTINGS_WATCH_DEBOUNCE_MS),
    )?;

    loop {
        // Settings changes first: an added/removed folder should be picked
        // up by the same pass, not wait for a separate later iteration.
        if let Some(events) = settings_watcher.recv_timeout(Duration::from_millis(300)) {
            if events
                .iter()
                .any(|p| p.file_name() == Some(settings_filename.as_os_str()))
            {
                let previous = config.clone();
                config = reload_settings(&settings_path, &config);
                on_settings_reload(&previous, &config);
                if config.file_watch_debounce_ms != previous.file_watch_debounce_ms {
                    folder_watcher
                        .set_debounce(Duration::from_millis(config.file_watch_debounce_ms.into()));
                }
                apply_folder_diff(
                    &db,
                    &extractors,
                    &bigram,
                    &mut kiwi,
                    &mut folder_watcher,
                    &mut watched,
                    &config,
                    &progress,
                );
            }
        }

        // "Reset Index" (tray menu, confirmed by the user before this fires):
        // wipe the DB, then re-scan every currently watched folder from
        // scratch. Unwatching first and clearing `watched` makes
        // `apply_folder_diff` treat every folder as newly added, reusing the
        // same scan/watch path a first-time folder add already goes through.
        if reset_rx.try_recv().is_ok() {
            eprintln!("Resetting index (Reset Index requested from tray)");
            match DocumentRepository::reset_all(&db.conn) {
                Ok(()) => {
                    for folder in &watched {
                        if let Err(e) = folder_watcher.unwatch(folder) {
                            eprintln!("Failed to unwatch {}: {e}", folder.display());
                        }
                    }
                    watched.clear();
                    apply_folder_diff(
                        &db,
                        &extractors,
                        &bigram,
                        &mut kiwi,
                        &mut folder_watcher,
                        &mut watched,
                        &config,
                        &progress,
                    );
                }
                Err(e) => eprintln!("Failed to reset index: {e}"),
            }
        }

        if let Some(events) = folder_watcher.recv_timeout(Duration::from_millis(300)) {
            // A path can still surface here for a folder that was *just*
            // unwatched (settings-triggered removal, or the unwatch half of
            // "Reset Index") - `notify`'s OS-level backend may have already
            // captured the event before the `unwatch()` call above took
            // effect, and once buffered, unwatching doesn't retroactively
            // discard it. Filtering against the current `watched` list (the
            // one `apply_folder_diff` just finished updating) is what
            // actually stops it from being indexed, rather than relying on
            // timing to keep the two from overlapping.
            let events: Vec<PathBuf> = events
                .into_iter()
                .filter(|path| watched.iter().any(|folder| path.starts_with(folder)))
                .collect();
            if !events.is_empty() {
                let pipeline = IndexPipeline {
                    conn: &db.conn,
                    config: &config,
                    extractors: &extractors,
                    bigram: &bigram,
                    kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
                };
                for (path, result) in queue::drain(&pipeline, events) {
                    if let Err(e) = result {
                        eprintln!("{}: index error: {e}", path.display());
                    }
                }
            }
        }
    }
}

/// "색인 중 (done/total)" (TASK-904, `docs/12_UI_Spec.md` C5) - `None` while
/// idle (nothing currently being scanned). Shared between the index worker
/// thread (writer, via `apply_folder_diff`) and the `get_index_progress`
/// command (reader, polled by the search window).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct IndexProgress {
    done: usize,
    total: usize,
}

type IndexProgressState = Arc<Mutex<Option<IndexProgress>>>;

/// Diffs `config.watched_folders` against `current` (updated in place to
/// match): newly listed folders get an initial scan and become watched;
/// removed ones stop being watched. Already-indexed documents from a removed
/// folder are left alone - not a purge (that's "색인 초기화", not built).
///
/// Lazily loads `kiwi` the first time any folder is ever added, so an app
/// with nothing configured yet doesn't pay Kiwi's memory cost for nothing
/// (`SearchWorker`'s doc comment has the full reasoning on why Kiwi can't
/// just be shared across threads instead).
///
/// While scanning newly-added folders, keeps `progress` updated with a
/// running total across all of them together (not reset per folder) - matters
/// most at startup, when every folder in `watched_folders` becomes "added" in
/// the same call, so the counter should read as one contiguous scan rather
/// than jumping back to 0 partway through.
#[allow(clippy::too_many_arguments)] // Already a private helper called from
                                     // exactly 3 sites inside `run_index_worker`, all right next to each other -
                                     // splitting these into a struct wouldn't make any of those call sites
                                     // clearer, just move the same fields one level of indirection away.
fn apply_folder_diff(
    db: &Db,
    extractors: &[Box<dyn ContentExtractor>],
    bigram: &BigramTokenizer,
    kiwi: &mut Option<KiwiTokenizer>,
    watcher: &mut FileWatcher,
    current: &mut Vec<PathBuf>,
    config: &Config,
    progress: &IndexProgressState,
) {
    let desired = &config.watched_folders;
    let added: Vec<&PathBuf> = desired.iter().filter(|f| !current.contains(f)).collect();
    let removed: Vec<&PathBuf> = current.iter().filter(|f| !desired.contains(f)).collect();
    if added.is_empty() && removed.is_empty() {
        return;
    }

    if !added.is_empty() && kiwi.is_none() {
        *kiwi = load_kiwi();
    }

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors,
        bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };
    // Scanned upfront (a plain directory walk, no file content read yet) so
    // the progress counter reflects the whole batch's total from the start,
    // not just whichever folder happens to be scanning right now.
    let total: usize = added.iter().map(|f| walker::scan(f).len()).sum();
    let mut done_so_far = 0;
    if total > 0 {
        *progress.lock().unwrap() = Some(IndexProgress { done: 0, total });
    }
    for folder in added {
        let base = done_so_far;
        let result = pipeline.index_directory_with_progress(folder, |done, _folder_total| {
            *progress.lock().unwrap() = Some(IndexProgress {
                done: base + done,
                total,
            });
        });
        match result {
            Ok(outcome) => {
                done_so_far += (outcome.full + outcome.meta + outcome.skip) as usize;
                eprintln!(
                    "Indexed {}: {} full-text, {} metadata, {} skipped",
                    folder.display(),
                    outcome.full,
                    outcome.meta,
                    outcome.skip
                )
            }
            Err(e) => {
                eprintln!("Failed to index {}: {e}", folder.display());
                continue;
            }
        }
        if let Err(e) = watcher.watch(folder) {
            eprintln!("Failed to watch {}: {e}", folder.display());
        }
    }
    *progress.lock().unwrap() = None; // Idle again - the whole batch is done.
    for folder in removed {
        if let Err(e) = watcher.unwatch(folder) {
            eprintln!("Failed to unwatch {}: {e}", folder.display());
        }
    }

    *current = desired.clone();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "macos")]
    set_bundled_native_lib_env_vars();

    let db_path = db_path();
    if let Some(parent) = db_path.parent() {
        // Best-effort - `Db::open` below fails loudly if this didn't work.
        let _ = std::fs::create_dir_all(parent);
    }
    let settings_path = settings_path();
    // First run: create `settings.json` with defaults so there's always a
    // real file for "Settings" (`open_settings_file`) to open, and so
    // hand-editing it means adding to something that already exists rather
    // than guessing the whole shape from scratch.
    if !settings_path.exists() {
        if let Err(e) = Config::default().save(&settings_path) {
            eprintln!("Failed to create default settings.json: {e}");
        }
    }
    let config = Config::load(Some(&settings_path)).unwrap_or_else(|e| {
        eprintln!("Failed to load settings, using defaults: {e}");
        Config::default()
    });
    // Opens (creating the file/schema/WAL mode if this is the first run) and
    // immediately drops a connection before `SearchWorker`/the index worker open
    // their own - otherwise two connections racing to switch a brand-new DB file
    // into WAL mode at the same moment can hit `SQLITE_BUSY` even with a
    // `busy_timeout` set (confirmed in practice: switching journal mode isn't
    // covered by the normal busy-retry path the way an ordinary read/write is).
    // Once the file/schema/WAL mode already exist, later connections opening
    // concurrently no longer hit this.
    Db::open(&db_path).expect("failed to open index DB");
    let worker = SearchWorker::spawn(db_path.clone());
    // "Reset Index" (tray menu) is sent over this channel rather than acted on
    // directly in the menu-event handler, since the DB connection it needs to
    // wipe belongs to the index worker thread, not the UI thread the tray
    // callback runs on.
    let (reset_tx, reset_rx) = mpsc::channel::<()>();
    // "색인 중 (done/total)" (TASK-904) - `.manage()`d below for
    // `get_index_progress` to read, and a clone moved into the index worker
    // thread (started in `.setup()`, once an `AppHandle` exists) to write.
    let index_progress: IndexProgressState = Arc::new(Mutex::new(None));

    tauri::Builder::default()
        // Must be registered first (upstream recommendation) - a second launch
        // attempt is caught here and the running instance's search window is
        // shown/focused instead of a new process starting.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_search_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(worker)
        .manage(index_progress.clone())
        .invoke_handler(tauri::generate_handler![
            search,
            open_path,
            open_parent_folder,
            open_settings_file,
            get_theme,
            get_result_limit,
            get_search_debounce_ms,
            preview_body,
            get_index_progress,
        ])
        .setup(move |app| {
            // Tray-only background app - no Dock icon, no Cmd+Tab entry.
            // `skipTaskbar` (`tauri.conf.json`) is the Windows/Linux equivalent;
            // macOS has no per-window taskbar flag, it's controlled by the whole
            // app's activation policy instead.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_handle = app.handle().clone();

            // "Statistics" (tray menu) opens its own short-lived connection
            // (like `knowdesk-cli stats`) rather than going through the index
            // worker thread - a plain read needs no coordination with it.
            let stats_db_path = db_path.clone();

            // Spawned here (not before `tauri::Builder::default()` above, as
            // originally written) because live hotkey/auto_start reload needs
            // an `AppHandle`, which doesn't exist until now.
            let reload_app = app_handle.clone();
            spawn_index_worker(
                db_path,
                settings_path,
                config.clone(),
                reset_rx,
                move |previous, current| {
                    if current.hotkey != previous.hotkey {
                        if let Err(e) = reload_app.global_shortcut().unregister(previous.hotkey.as_str())
                        {
                            eprintln!("Failed to unregister old hotkey {}: {e}", previous.hotkey);
                        }
                        match register_hotkey(&reload_app, &current.hotkey) {
                            Ok(()) => eprintln!(
                                "Hotkey changed: {} -> {}",
                                previous.hotkey, current.hotkey
                            ),
                            Err(e) => eprintln!(
                                "Failed to register new hotkey {}: {e}",
                                current.hotkey
                            ),
                        }
                    }
                    if current.auto_start != previous.auto_start {
                        sync_autostart(&reload_app, current.auto_start);
                    }
                },
                index_progress,
            );

            // The tray is the only thing keeping the app around once the
            // window is hidden, so its OS-level close request (e.g. Alt+F4)
            // must hide the window instead of destroying it - same intent as
            // `Esc` (`docs/12_UI_Spec.md` C1: "창 닫기(트레이로 숨김, 프로세스
            // 종료 아님)"), just reached through a different trigger.
            if let Some(window) = app.get_webview_window("search") {
                let window_to_hide = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_to_hide.hide();
                    }
                });
            }

            // English labels, per request - unlike the rest of the app's UI
            // (Korean), this menu's wording was explicitly specified in English.
            // No separate "Reload" item - settings.json is now watched like any
            // other indexed folder (`run_index_worker`), so edits (and deletes,
            // which recreate it with defaults) apply on their own. "Reset Index"
            // gets its own separators on both sides - it's destructive
            // (wipes the whole DB), unlike "Settings"/"Quit".
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let statistics_item =
                MenuItem::with_id(app, "statistics", "Statistics", true, None::<&str>)?;
            let separator_1 = PredefinedMenuItem::separator(app)?;
            let reset_index_item =
                MenuItem::with_id(app, "reset_index", "Reset Index", true, None::<&str>)?;
            let separator_2 = PredefinedMenuItem::separator(app)?;
            // `tauri dev` runs the binary outside a proper .app bundle, so
            // there's no Info.plist-declared icon for the OS to fall back
            // on - without an explicit icon here, the About panel shows a
            // generic placeholder (a plain folder icon) instead of KnowDesk's
            // icon. name/version are left unset (`..Default::default()`) so
            // the OS fills those in from the bundle itself once packaged,
            // rather than us duplicating a value it already knows.
            // `copyright` is the one AboutMetadata field every platform's
            // native About panel renders (macOS's `authors`/`comments` and
            // Windows/Linux's `credits` are each unsupported on some
            // platform) - reused here to show developer credit as a plain
            // line under the version, the same everywhere.
            let about_icon = Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            let about_item = PredefinedMenuItem::about(
                app,
                Some("About"),
                Some(AboutMetadata {
                    icon: Some(about_icon),
                    copyright: Some("Developed by Yonghee Yu".to_string()),
                    ..Default::default()
                }),
            )?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(
                app,
                &[
                    &settings_item,
                    &statistics_item,
                    &separator_1,
                    &reset_index_item,
                    &separator_2,
                    &about_item,
                    &quit_item,
                ],
            )?;

            // macOS gets its own monochrome silhouette + alpha icon
            // (`tray_light.png`) so `.icon_as_template(true)` below can make
            // the menu bar recolor it for light/dark mode automatically -
            // that only works with a template-style image, not a colored
            // one. Windows/Ubuntu have no such OS-level auto-recoloring
            // (`icon_as_template` is a no-op there), so they keep the plain
            // colored icon for now - pending `tray_dark.png`, a matching
            // pair for manually switching by detected system theme.
            #[cfg(target_os = "macos")]
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray_light.png"))?;
            #[cfg(not(target_os = "macos"))]
            let tray_icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
            TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(true)
                .tooltip("KnowDesk")
                .menu(&tray_menu)
                // Left click is handled ourselves below (toggle the search
                // window); only right click opens the menu
                // (`docs/12_UI_Spec.md` C4: 좌클릭=토글, 우클릭=메뉴).
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "settings" => {
                        let _ = open_settings_file(app.clone());
                    }
                    "statistics" => {
                        let text = compute_stats(&stats_db_path)
                            .unwrap_or_else(|e| format!("Failed to read statistics: {e}"));
                        app.dialog()
                            .message(text)
                            .title("Statistics")
                            .kind(MessageDialogKind::Info)
                            .show(|_| {});
                    }
                    "reset_index" => {
                        let reset_tx = reset_tx.clone();
                        // Destructive and irreversible (wipes every indexed
                        // document) - confirmed with a native dialog before
                        // doing anything. Uses the non-blocking `.show()`
                        // callback form, never a `blocking_*` one - see the
                        // `tauri-plugin-dialog` dependency comment for why
                        // that distinction matters here.
                        app.dialog()
                            .message(
                                "This deletes the entire search index and re-scans every watched folder from scratch. This cannot be undone.",
                            )
                            .title("Reset Index")
                            .kind(MessageDialogKind::Warning)
                            .buttons(MessageDialogButtons::OkCancelCustom(
                                "Reset".to_string(),
                                "Cancel".to_string(),
                            ))
                            .show(move |confirmed| {
                                if confirmed {
                                    let _ = reset_tx.send(());
                                }
                            });
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_search_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // TASK-802: registered here (app-level setup), not via the
            // plugin builder's `.with_shortcut()` - that runs before the app
            // handle exists, so a parse failure there can't surface through
            // the normal `?` error path the way it can here.
            register_hotkey(&app_handle, &config.hotkey)?;

            // Reconciles the actual OS login-item state with `auto_start` on
            // every startup, not just when it changes - e.g. it may have
            // been enabled by hand outside the app, or a previous run may
            // have failed to apply it.
            sync_autostart(&app_handle, config.auto_start);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowdesk_core::config::Config;
    use knowdesk_core::db::documents::{DocumentRecord, DocumentRepository, IndexTier};
    use knowdesk_core::extract::txt::TxtExtractor;
    use knowdesk_core::extract::ContentExtractor;
    use knowdesk_core::index::pipeline::IndexPipeline;
    use knowdesk_core::nlp::bigram::BigramTokenizer;

    /// Serializes the tests that spawn `run_index_worker`, which creates real
    /// `notify` file watchers - confirmed in practice that two such watchers
    /// running concurrently (the default when `cargo test` runs tests in
    /// parallel) can starve each other's FSEvents callback delivery on macOS,
    /// making both tests flaky for reasons that have nothing to do with the
    /// code under test (same root cause as the fixed-sleep-over-tight-polling
    /// note on `index_worker_applies_settings_file_changes_live` below - this
    /// closes the other half of the gap, now that there's more than one such
    /// test). `search_worker_finds_indexed_document` doesn't create a file
    /// watcher, so it's unaffected and stays unguarded.
    static INDEX_WORKER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// End-to-end check of the `SearchWorker` channel plumbing (the part unit
    /// tests can't reach through `#[tauri::command]` alone): index a real file
    /// on disk, then verify a search sent across the worker thread's channel
    /// gets the expected hit back.
    #[test]
    fn search_worker_finds_indexed_document() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("규정.txt"), "채권 발행 절차를 규정한다.").unwrap();
        let db_path = dir.path().join("test.db");

        {
            let db = Db::open(&db_path).unwrap();
            let config = Config::default();
            let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
            let bigram = BigramTokenizer;
            let pipeline = IndexPipeline {
                conn: &db.conn,
                config: &config,
                extractors: &extractors,
                bigram: &bigram,
                kiwi: None,
            };
            pipeline.index_directory(dir.path()).unwrap();
        }

        let worker = SearchWorker::spawn(db_path);
        let hits = worker
            .search("채권".to_string(), CoreSearchMode::Content, 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].filename, "규정.txt");
        assert_eq!(hits[0].match_kind, "exact");
    }

    fn search_finds(db: &Db, query: &str) -> bool {
        let service = SqliteSearchService {
            conn: &db.conn,
            kiwi: None,
        };
        service
            .search(&SearchRequest {
                query: query.to_string(),
                mode: CoreSearchMode::Content,
                limit: 10,
            })
            .map(|r| !r.hits.is_empty())
            .unwrap_or(false)
    }

    /// End-to-end check of `run_index_worker`'s whole point: applying
    /// `settings.json` changes live, no restart or manual "reload" action.
    /// Covers all three cases from the tray menu's "Reload" removal - edit
    /// (add a folder), and delete (reset to defaults, stop watching).
    ///
    /// Uses fixed sleeps rather than a tight poll loop for each wait -
    /// confirmed in practice that polling by repeatedly querying/opening a
    /// connection to the same DB file the worker thread is also using can
    /// starve the `notify`/FSEvents callback its watchers depend on, making
    /// this test flaky for reasons that have nothing to do with the code
    /// under test. Each wait below is generous relative to the worker's own
    /// 3s debounce.
    #[test]
    fn index_worker_applies_settings_file_changes_live() {
        let _guard = INDEX_WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings_path = dir.path().join("settings.json");
        let watched_a = dir.path().join("watched_a");
        let watched_b = dir.path().join("watched_b");
        std::fs::create_dir_all(&watched_a).unwrap();
        std::fs::create_dir_all(&watched_b).unwrap();
        std::fs::write(watched_a.join("규정.txt"), "채권 발행 절차를 규정한다.").unwrap();

        let config = Config {
            watched_folders: vec![watched_a.clone()],
            ..Config::default()
        };
        config.save(&settings_path).unwrap();

        // Same WAL-init race avoidance as `run()` (`Db::open` before spawning
        // any worker that opens its own connection).
        Db::open(&db_path).unwrap();
        let (_reset_tx, reset_rx) = mpsc::channel();
        spawn_index_worker(
            db_path.clone(),
            settings_path.clone(),
            config,
            reset_rx,
            |_, _| {},
            Arc::new(Mutex::new(None)),
        );

        std::thread::sleep(std::time::Duration::from_secs(2));
        let search_db = Db::open(&db_path).unwrap();
        assert!(
            search_finds(&search_db, "채권"),
            "initial scan did not pick up the first watched folder"
        );
        drop(search_db);

        // Hand-edit settings.json to add a second folder - must apply on its
        // own, with no explicit reload call anywhere in this test.
        std::fs::write(
            watched_b.join("결의.txt"),
            "이사회 결의를 통해 예산을 승인했다.",
        )
        .unwrap();
        let updated = Config {
            watched_folders: vec![watched_a.clone(), watched_b.clone()],
            ..Config::default()
        };
        updated.save(&settings_path).unwrap();

        std::thread::sleep(std::time::Duration::from_secs(6));
        let search_db = Db::open(&db_path).unwrap();
        assert!(
            search_finds(&search_db, "예산"),
            "adding a folder to settings.json was not applied automatically"
        );
        drop(search_db);

        // Deleting settings.json must recreate it with defaults (empty
        // watched_folders) and stop watching both folders.
        std::fs::remove_file(&settings_path).unwrap();
        std::fs::write(watched_a.join("새문서.txt"), "고유표시자12345").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(6));

        assert!(
            settings_path.exists(),
            "settings.json was not recreated after being deleted"
        );
        let search_db = Db::open(&db_path).unwrap();
        assert!(
            !search_finds(&search_db, "고유표시자12345"),
            "a folder must stop being watched once settings.json resets to defaults"
        );
    }

    /// End-to-end check of the "Reset Index" tray action's channel-driven
    /// trigger: a signal on `reset_rx` must wipe the DB and re-scan every
    /// watched folder from scratch. Planting a document with no backing file
    /// before the trigger fires distinguishes an actual wipe from the rescan
    /// simply re-upserting the same real documents it would have anyway.
    #[test]
    fn index_worker_resets_on_reset_channel_trigger() {
        let _guard = INDEX_WORKER_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let settings_path = dir.path().join("settings.json");
        let watched = dir.path().join("watched");
        std::fs::create_dir_all(&watched).unwrap();
        std::fs::write(watched.join("규정.txt"), "채권 발행 절차를 규정한다.").unwrap();

        let config = Config {
            watched_folders: vec![watched.clone()],
            ..Config::default()
        };
        config.save(&settings_path).unwrap();

        let db = Db::open(&db_path).unwrap();
        DocumentRepository::upsert_document(
            &db.conn,
            &DocumentRecord {
                document_id: "orphan".to_string(),
                file_size: 1,
                text_bytes: 1,
                index_tier: IndexTier::Meta,
            },
        )
        .unwrap();
        drop(db);

        let (reset_tx, reset_rx) = mpsc::channel();
        spawn_index_worker(
            db_path.clone(),
            settings_path.clone(),
            config,
            reset_rx,
            |_, _| {},
            Arc::new(Mutex::new(None)),
        );

        std::thread::sleep(std::time::Duration::from_secs(2));
        let check_db = Db::open(&db_path).unwrap();
        assert!(
            search_finds(&check_db, "채권"),
            "initial scan did not index the watched folder"
        );
        assert!(
            DocumentRepository::exists(&check_db.conn, "orphan").unwrap(),
            "orphan document should still be present before reset"
        );
        drop(check_db);

        reset_tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2));

        let check_db = Db::open(&db_path).unwrap();
        assert!(
            !DocumentRepository::exists(&check_db.conn, "orphan").unwrap(),
            "Reset Index did not wipe the previous index"
        );
        assert!(
            search_finds(&check_db, "채권"),
            "Reset Index did not re-scan the watched folder"
        );
    }

    #[test]
    fn compute_stats_reports_tier_counts_and_db_size() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        {
            let db = Db::open(&db_path).unwrap();
            DocumentRepository::upsert_document(
                &db.conn,
                &DocumentRecord {
                    document_id: "full1".to_string(),
                    file_size: 10,
                    text_bytes: 5,
                    index_tier: IndexTier::Full,
                },
            )
            .unwrap();
            DocumentRepository::upsert_document(
                &db.conn,
                &DocumentRecord {
                    document_id: "meta1".to_string(),
                    file_size: 10,
                    text_bytes: 0,
                    index_tier: IndexTier::Meta,
                },
            )
            .unwrap();
        }

        let stats = compute_stats(&db_path).unwrap();
        assert!(stats.contains("Total: 2 documents"), "{stats}");
        assert!(stats.contains("Full text indexed: 1"), "{stats}");
        assert!(stats.contains("Metadata only: 1"), "{stats}");
        assert!(!stats.contains("Last indexed: never"), "{stats}");
    }

    #[test]
    fn compute_stats_on_empty_db_reports_zero_and_never_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        Db::open(&db_path).unwrap();

        let stats = compute_stats(&db_path).unwrap();
        assert!(stats.contains("Total: 0 documents"), "{stats}");
        assert!(stats.contains("Last indexed: never"), "{stats}");
    }

    /// `apply_folder_diff` scanning two newly-added folders at once (the
    /// startup case - every folder in `watched_folders` is "added" in the
    /// same call) must index both and leave `progress` idle (`None`)
    /// afterward, not stuck showing a stale "done/total" from mid-scan
    /// (TASK-904).
    #[test]
    fn apply_folder_diff_indexes_multiple_added_folders_and_clears_progress() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(dir_a.path().join("a1.txt"), "채권 발행").unwrap();
        std::fs::write(dir_a.path().join("a2.txt"), "이사회 결의").unwrap();
        std::fs::write(dir_b.path().join("b1.txt"), "예산 승인").unwrap();

        let db = Db::open_in_memory().unwrap();
        let extractors = default_extractors();
        let bigram = BigramTokenizer;
        let mut kiwi: Option<KiwiTokenizer> = None;
        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        let config = Config {
            watched_folders: vec![dir_a.path().to_path_buf(), dir_b.path().to_path_buf()],
            ..Config::default()
        };
        let progress: IndexProgressState = Arc::new(Mutex::new(None));

        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &mut kiwi,
            &mut watcher,
            &mut watched,
            &config,
            &progress,
        );

        assert_eq!(
            *progress.lock().unwrap(),
            None,
            "must be idle again once the whole batch finishes, not left mid-scan"
        );
        assert_eq!(watched.len(), 2);
        let tiers = DocumentRepository::count_by_tier(&db.conn).unwrap();
        let full = tiers
            .iter()
            .find(|(tier, _)| tier == "FULL")
            .map_or(0, |(_, count)| *count);
        assert_eq!(full, 3, "all 3 files across both folders should be indexed");
    }
}
