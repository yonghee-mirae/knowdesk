// Thin IPC bindings only - every command delegates to the `knowdesk-core` crate
// (`CLAUDE.md`: "core는 Tauri를 절대 참조하지 않는다. 모든 OS 통합은 src-tauri로 격리한다").
// `open_path`/`open_parent_folder` are the exception (native opener, no equivalent in `core`).

use knowdesk_core::config::{Config, Theme};
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
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{
    MatchKind, SearchHit, SearchMode as CoreSearchMode, SearchRequest,
    SearchService as SearchServiceTrait,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

/// Not yet user-changeable since there's no Settings Window (TASK-704 —
/// replaced with a "설정 파일 폴더 열기" action, see `open_settings_folder`)
/// to change it from. `CmdOrCtrl` resolves to `⌘+Option+K` on macOS,
/// `Ctrl+Alt+K` on Windows/Linux.
///
/// ⚠️ O-7 (`KnowDesk_추가검토사항.md`) - whether global-hotkey hooking is
/// allowed by IT security policy on the target Windows fleet - is still
/// unconfirmed. This default is provisional pending that review.
const DEFAULT_HOTKEY: &str = "CmdOrCtrl+Alt+K";

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
/// (`README.md`'s `--db`-less CLI behavior is cwd-relative, which isn't
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
/// replaced with `open_settings_folder`, which just opens this file's folder) -
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

/// "설정" action (tray menu / search bar gear icon): there's no in-app
/// Settings window (TASK-704) - opens the folder containing `settings.json`
/// in the OS file manager instead, so editing that file directly (in any
/// text editor) is the whole UI. `run()` makes sure the file actually exists
/// with defaults before this could ever be called, so there's always
/// something real to show here.
#[tauri::command]
fn open_settings_folder(app: AppHandle) -> Result<(), String> {
    let path = settings_path();
    let folder = path
        .parent()
        .ok_or_else(|| "No parent folder".to_string())?;
    app.opener()
        .open_path(folder.to_string_lossy().into_owned(), None::<String>)
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

/// Shows the "search" window (pre-created hidden at startup, `tauri.conf.json`'s
/// `visible: false`) and gives it keyboard focus, or hides it again if it's
/// already visible (Spotlight/PowerToys Run convention -
/// `docs/12_UI_Spec.md` C4: "이미 열려 있는 검색창에서 단축키를 다시 누르면 →
/// 토글(다시 누르면 닫힘)"). Shared by the tray icon's left click and the
/// global hotkey.
fn toggle_search_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("search") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
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
fn spawn_index_worker(db_path: PathBuf, settings_path: PathBuf, config: Config) {
    thread::spawn(move || {
        if let Err(e) = run_index_worker(db_path, settings_path, config) {
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
/// one more file to apply the same idea to.
fn run_index_worker(
    db_path: PathBuf,
    settings_path: PathBuf,
    initial_config: Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = Db::open(&db_path)?;
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let mut kiwi: Option<KiwiTokenizer> = None;
    let mut watched: Vec<PathBuf> = Vec::new();
    let mut config = initial_config;
    // Same debounce as `knowdesk-cli watch`'s default (`cli/src/main.rs`).
    // Starts with no roots - `apply_folder_diff` below adds whatever
    // `initial_config` lists.
    let mut folder_watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000))?;
    apply_folder_diff(
        &db,
        &extractors,
        &bigram,
        &mut kiwi,
        &mut folder_watcher,
        &mut watched,
        &config,
    );

    // Watches `settings.json`'s own folder (not the file directly - `notify`
    // watches on some platforms don't survive the file being deleted and
    // recreated, which is exactly the "삭제 시 기본값으로 재생성" case this
    // needs to catch) so a hand-edit or delete applies on its own.
    let settings_filename = settings_path
        .file_name()
        .ok_or("settings path has no file name")?
        .to_owned();
    let settings_dir = settings_path
        .parent()
        .ok_or("settings path has no parent directory")?;
    let settings_watcher = FileWatcher::new(&[settings_dir], Duration::from_millis(3000))?;

    loop {
        // Settings changes first: an added/removed folder should be picked
        // up by the same pass, not wait for a separate later iteration.
        if let Some(events) = settings_watcher.recv_timeout(Duration::from_millis(300)) {
            if events
                .iter()
                .any(|p| p.file_name() == Some(settings_filename.as_os_str()))
            {
                config = reload_settings(&settings_path, &config);
                apply_folder_diff(
                    &db,
                    &extractors,
                    &bigram,
                    &mut kiwi,
                    &mut folder_watcher,
                    &mut watched,
                    &config,
                );
            }
        }

        if let Some(events) = folder_watcher.recv_timeout(Duration::from_millis(300)) {
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

/// Diffs `config.watched_folders` against `current` (updated in place to
/// match): newly listed folders get an initial scan and become watched;
/// removed ones stop being watched. Already-indexed documents from a removed
/// folder are left alone - not a purge (that's "색인 초기화", not built).
///
/// Lazily loads `kiwi` the first time any folder is ever added, so an app
/// with nothing configured yet doesn't pay Kiwi's memory cost for nothing
/// (`SearchWorker`'s doc comment has the full reasoning on why Kiwi can't
/// just be shared across threads instead).
fn apply_folder_diff(
    db: &Db,
    extractors: &[Box<dyn ContentExtractor>],
    bigram: &BigramTokenizer,
    kiwi: &mut Option<KiwiTokenizer>,
    watcher: &mut FileWatcher,
    current: &mut Vec<PathBuf>,
    config: &Config,
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
    for folder in added {
        match pipeline.index_directory(folder) {
            Ok(outcome) => eprintln!(
                "Indexed {}: {} full-text, {} metadata, {} skipped",
                folder.display(),
                outcome.full,
                outcome.meta,
                outcome.skip
            ),
            Err(e) => {
                eprintln!("Failed to index {}: {e}", folder.display());
                continue;
            }
        }
        if let Err(e) = watcher.watch(folder) {
            eprintln!("Failed to watch {}: {e}", folder.display());
        }
    }
    for folder in removed {
        if let Err(e) = watcher.unwatch(folder) {
            eprintln!("Failed to unwatch {}: {e}", folder.display());
        }
    }

    *current = desired.clone();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = db_path();
    if let Some(parent) = db_path.parent() {
        // Best-effort - `Db::open` below fails loudly if this didn't work.
        let _ = std::fs::create_dir_all(parent);
    }
    let settings_path = settings_path();
    // First run: create `settings.json` with defaults so there's always a
    // real file for "설정" (`open_settings_folder`) to point at, and so
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
    spawn_index_worker(db_path.clone(), settings_path, config);
    let worker = SearchWorker::spawn(db_path);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(worker)
        .invoke_handler(tauri::generate_handler![
            search,
            open_path,
            open_parent_folder,
            open_settings_folder,
            get_theme,
        ])
        .setup(|app| {
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
            // which recreate it with defaults) apply on their own.
            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&settings_item, &separator, &quit_item])?;

            let tray_icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
            TrayIconBuilder::new()
                .icon(tray_icon)
                .tooltip("KnowDesk")
                .menu(&tray_menu)
                // Left click is handled ourselves below (toggle the search
                // window); only right click opens the menu
                // (`docs/12_UI_Spec.md` C4: 좌클릭=토글, 우클릭=메뉴).
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => {
                        let _ = open_settings_folder(app.clone());
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
            app.global_shortcut()
                .on_shortcut(DEFAULT_HOTKEY, |app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_search_window(app);
                    }
                })?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowdesk_core::config::Config;
    use knowdesk_core::extract::txt::TxtExtractor;
    use knowdesk_core::extract::ContentExtractor;
    use knowdesk_core::index::pipeline::IndexPipeline;
    use knowdesk_core::nlp::bigram::BigramTokenizer;

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
        spawn_index_worker(db_path.clone(), settings_path.clone(), config);

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
}
