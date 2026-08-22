// Thin IPC bindings only - every command delegates to the `knowdesk-core` crate
// (`CLAUDE.md`: "core는 Tauri를 절대 참조하지 않는다. 모든 OS 통합은 src-tauri로 격리한다").
// `open_path`/`open_parent_folder` are the exception (native opener, no equivalent in `core`).

use knowdesk_core::config::Config;
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
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_opener::OpenerExt;

/// Not yet user-changeable since there's no Settings Window (TASK-704) to
/// change it from. `CmdOrCtrl` resolves to `⌘+Option+K` on macOS,
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
/// `KNOWDESK_DB_PATH` (`db_path()`). There's no Settings Window yet (TASK-704) to
/// write this file through, so until then a missing file (the common case) just
/// falls back to `Config::default()` - i.e. no folders watched, nothing indexed.
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

/// Shows the "search" window (pre-created hidden at startup, `tauri.conf.json`'s
/// `visible: false`) and gives it keyboard focus - the single "reveal" action
/// shared by the tray icon's left click and its "검색창 열기" menu item
/// (`docs/12_UI_Spec.md` C4).
fn show_search_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("search") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The global hotkey's action: shows the search window, or hides it again if
/// it's already visible (Spotlight/PowerToys Run convention -
/// `docs/12_UI_Spec.md` C4: "이미 열려 있는 검색창에서 단축키를 다시 누르면 →
/// 토글(다시 누르면 닫힘)"). Unlike the tray icon's left click
/// (`show_search_window`), which always just reveals it.
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

/// Keeps every folder in `Config::watched_folders` indexed for the rest of the
/// process's lifetime: an initial full scan, then continuous watching. A no-op
/// if the list is empty (the common case today - see `settings_path()`).
///
/// One thread total, not one per folder: `FileWatcher::new` accepts multiple
/// roots on a single underlying watcher (`core/src/index/watcher.rs`), which
/// is what lets this stay a single thread with a single `KiwiTokenizer`
/// instance - Kiwi isn't `Send` (see `SearchWorker`'s doc comment above), so N
/// folders on N threads would mean N separate Kiwi models loaded at once.
fn spawn_index_worker(db_path: PathBuf, config: Config) {
    if config.watched_folders.is_empty() {
        return;
    }
    thread::spawn(move || {
        if let Err(e) = run_index_worker(&db_path, &config) {
            eprintln!("Index worker failed: {e}");
        }
    });
}

fn run_index_worker(db_path: &Path, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let db = Db::open(db_path)?;
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
    let kiwi = load_kiwi();
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: kiwi.as_ref().map(|k| k as &dyn Tokenizer),
    };

    for folder in &config.watched_folders {
        let outcome = pipeline.index_directory(folder)?;
        eprintln!(
            "Indexed {}: {} full-text, {} metadata, {} skipped",
            folder.display(),
            outcome.full,
            outcome.meta,
            outcome.skip
        );
    }

    // Same debounce as `knowdesk-cli watch`'s default (`cli/src/main.rs`).
    let watcher = FileWatcher::new(&config.watched_folders, Duration::from_millis(3000))?;
    while let Some(events) = watcher.recv() {
        for (path, result) in queue::drain(&pipeline, events) {
            if let Err(e) = result {
                eprintln!("{}: index error: {e}", path.display());
            }
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = db_path();
    if let Some(parent) = db_path.parent() {
        // Best-effort - `Db::open` below fails loudly if this didn't work.
        let _ = std::fs::create_dir_all(parent);
    }
    let config = Config::load(Some(&settings_path())).unwrap_or_else(|e| {
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
    spawn_index_worker(db_path.clone(), config);
    let worker = SearchWorker::spawn(db_path);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(worker)
        .invoke_handler(tauri::generate_handler![
            search,
            open_path,
            open_parent_folder
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

            let show_item = MenuItem::with_id(app, "show", "검색창 열기", true, None::<&str>)?;
            // TASK-704(Settings Window)가 아직 없어 비활성화만 해둔다 - 화면이
            // 생기면 `enabled(true)` + `on_menu_event`에 "settings" 분기만
            // 추가하면 된다.
            let settings_item = MenuItem::with_id(app, "settings", "설정", false, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show_item, &settings_item, &quit_item])?;

            let tray_icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
            TrayIconBuilder::new()
                .icon(tray_icon)
                .tooltip("KnowDesk")
                .menu(&tray_menu)
                // Left click is handled ourselves below (show the search
                // window); only right click opens the menu
                // (`docs/12_UI_Spec.md` C4: 좌클릭=표시, 우클릭=메뉴).
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_search_window(app),
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
                        show_search_window(tray.app_handle());
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
}
