// Thin IPC bindings only - every command delegates to the `knowdesk-core` crate
// (`CLAUDE.md`: "core는 Tauri를 절대 참조하지 않는다. 모든 OS 통합은 src-tauri로 격리한다").
// `open_path`/`open_parent_folder` are the exception (native opener, no equivalent in `core`).

use knowdesk_core::config::{Config, Theme};
use knowdesk_core::db::documents::DocumentRepository;
use knowdesk_core::db::search_repo::SearchRepository;
use knowdesk_core::db::Db;
use knowdesk_core::extract::ooxml::{DocxExtractor, PptxExtractor};
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::xlsx::XlsxExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::canonical_path;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::index::queue;
use knowdesk_core::index::watcher::FileWatcher;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::nlp::{Token, Tokenizer};
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
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
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

/// Confines the DB connection to one dedicated thread and talks to it over a
/// channel, rather than sharing it behind a `Mutex` in Tauri's managed state.
/// The Kiwi tokenizer itself is *not* owned here - see `KiwiHandle`, shared
/// with the index/watch worker so the two don't each load their own copy of
/// Kiwi's large in-memory model.
struct SearchWorker {
    sender: mpsc::Sender<SearchJob>,
}

impl SearchWorker {
    fn spawn(db_path: PathBuf, kiwi: KiwiHandle) -> Self {
        let (sender, receiver) = mpsc::channel::<SearchJob>();
        thread::spawn(move || {
            let db = Db::open(&db_path).expect("failed to open index DB");
            for job in receiver {
                // Read fresh per search, same "no cache, just re-read
                // settings.json" pattern as `get_theme`/`get_result_limit` -
                // so flipping `enable_morphological_analysis` off/on and
                // saving takes effect on the very next search, no restart.
                // `&&` short-circuits `ensure_loaded()` (Kiwi's load attempt)
                // away entirely while it's off.
                let kiwi_available = Config::load(Some(&settings_path()))
                    .map(|c| c.enable_morphological_analysis)
                    .unwrap_or(false)
                    && kiwi.ensure_loaded();
                let service = SqliteSearchService {
                    conn: &db.conn,
                    kiwi: if kiwi_available {
                        Some(&kiwi as &dyn Tokenizer)
                    } else {
                        None
                    },
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

/// A tokenize/locate request sent to `KiwiActor`'s thread, plus a reply channel
/// for the result. `EnsureLoaded` triggers the one-time load attempt
/// (`load_kiwi()`) if it hasn't happened yet, and is otherwise a cheap no-op
/// round trip - both call sites (`SearchWorker`, `apply_folder_diff`) use it to
/// find out whether Kiwi ended up available before building their
/// `Option<&dyn Tokenizer>` for that job.
enum KiwiJob {
    EnsureLoaded {
        reply: mpsc::Sender<bool>,
    },
    Tokenize {
        text: String,
        reply: mpsc::Sender<Vec<Token>>,
    },
    Locate {
        text: String,
        forms: Vec<String>,
        reply: mpsc::Sender<Option<(usize, usize)>>,
    },
}

/// Confines the single shared `KiwiTokenizer` instance to one dedicated
/// thread, for the same reason `SearchWorker` confines its DB connection:
/// `kiwi_rs::Kiwi` isn't `Send` (its internal caches hold `Box<dyn Fn>` rule
/// callbacks), so it must stay on the one thread that created it for its
/// entire lifetime. Previously `SearchWorker` and the index/watch worker each
/// called `load_kiwi()` themselves, so each held its own full copy of Kiwi's
/// model in memory - measured at ~824MB RSS per instance, doubling to ~1.6GB
/// once any folder was watched. Both now go through this one actor's
/// `KiwiHandle` instead, cutting that back down to a single instance.
struct KiwiActor;

impl KiwiActor {
    fn spawn() -> KiwiHandle {
        let (sender, receiver) = mpsc::channel::<KiwiJob>();
        thread::spawn(move || {
            let mut kiwi: Option<KiwiTokenizer> = None;
            let mut load_attempted = false;
            for job in receiver {
                if !load_attempted {
                    load_attempted = true;
                    kiwi = load_kiwi();
                }
                match job {
                    KiwiJob::EnsureLoaded { reply } => {
                        let _ = reply.send(kiwi.is_some());
                    }
                    KiwiJob::Tokenize { text, reply } => {
                        let tokens = kiwi.as_ref().map(|k| k.tokenize(&text)).unwrap_or_default();
                        let _ = reply.send(tokens);
                    }
                    KiwiJob::Locate { text, forms, reply } => {
                        let span = kiwi.as_ref().and_then(|k| k.locate(&text, &forms));
                        let _ = reply.send(span);
                    }
                }
            }
        });
        KiwiHandle { sender }
    }
}

/// Cloneable, `Send` handle to `KiwiActor`'s thread - stands in for a
/// `KiwiTokenizer` reference at each of the two call sites that used to hold
/// their own instance. Implements `Tokenizer` itself so it can be used
/// anywhere `&dyn Tokenizer` is expected, forwarding each call to the actor
/// thread and blocking on the reply (the same round-trip pattern
/// `SearchWorker::search` already uses for its own channel).
#[derive(Clone)]
struct KiwiHandle {
    sender: mpsc::Sender<KiwiJob>,
}

impl KiwiHandle {
    /// Returns whether Kiwi is available (configured and loaded successfully),
    /// triggering the load attempt first if it hasn't happened yet.
    fn ensure_loaded(&self) -> bool {
        let (reply, rx) = mpsc::channel();
        if self.sender.send(KiwiJob::EnsureLoaded { reply }).is_err() {
            return false;
        }
        rx.recv().unwrap_or(false)
    }
}

impl Tokenizer for KiwiHandle {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let (reply, rx) = mpsc::channel();
        if self
            .sender
            .send(KiwiJob::Tokenize {
                text: text.to_string(),
                reply,
            })
            .is_err()
        {
            return Vec::new();
        }
        rx.recv().unwrap_or_default()
    }

    fn locate(&self, text: &str, forms: &[String]) -> Option<(usize, usize)> {
        let (reply, rx) = mpsc::channel();
        if self
            .sender
            .send(KiwiJob::Locate {
                text: text.to_string(),
                forms: forms.to_vec(),
                reply,
            })
            .is_err()
        {
            return None;
        }
        rx.recv().ok().flatten()
    }
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

/// Windows equivalent of the macOS function above - points the same three env
/// vars at the native libraries/model bundled by the installer (see
/// `tauri.windows.conf.json`'s `bundle.resources` and `docs/03_Architecture.md`).
/// Same call site, same rationale (runs before an `AppHandle` exists).
///
/// Unlike macOS (`Contents/Resources`) and Linux's `.deb` (`/usr/lib/<name>`),
/// Tauri's own `resource_dir()` resolves to the **exe's own directory** on
/// Windows (`tauri_utils::platform::resource_dir_from`: "Windows also
/// includes the resources in the executable folder") - the installer places
/// `bundle.resources` right next to the installed `.exe`, no subfolder jump.
///
/// ⚠️ **Unverified - no Windows machine available to test this.** Filenames/
/// layout follow the same assumptions already documented in `env.ps1`:
/// `pdfium.dll` under a `bin/` folder (pdfium-binaries' Windows release layout
/// - unconfirmed, mac/Linux release layouts use `lib/` instead) and `kiwi.dll`
/// directly under `lib/` (confirmed via `scripts/install_kiwi.ps1` per
/// `env.ps1`, no `lib` prefix on the filename unlike `libkiwi.{so,dylib}`).
/// Verify against a real packaged build before relying on this.
///
/// Same graceful fallback as macOS: an already-set env var wins, and this is
/// a no-op if the bundled files aren't there - e.g. `cargo run`/`tauri dev`,
/// which has no `native/` folder next to the dev binary (local dev uses
/// `env.ps1`'s `.pdfium`/`.kiwi` paths instead).
#[cfg(target_os = "windows")]
fn set_bundled_native_lib_env_vars() {
    let Some(resources) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("native")))
    else {
        return;
    };

    if std::env::var_os("KNOWDESK_PDFIUM_LIB_DIR").is_none() {
        let dir = resources.join("pdfium");
        if dir.join("pdfium.dll").is_file() {
            // SAFETY: called once, single-threaded, before any thread that
            // could read the environment concurrently is spawned.
            unsafe { std::env::set_var("KNOWDESK_PDFIUM_LIB_DIR", dir) };
        }
    }

    if std::env::var_os("KNOWDESK_KIWI_LIB_PATH").is_none()
        && std::env::var_os("KNOWDESK_KIWI_MODEL_DIR").is_none()
    {
        let lib_path = resources.join("kiwi/kiwi.dll");
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

/// Whether Kiwi's morphological analysis is actually usable right now -
/// `enable_morphological_analysis` (`settings.json`) is on *and* Kiwi itself
/// initialized successfully (`KiwiHandle::ensure_loaded`, triggering the
/// one-time load attempt if it hasn't happened yet). Same "read on load +
/// window focus" pattern as `get_theme` above - like a settings value, this
/// doesn't change while the window just sits open. `&&` short-circuits
/// `ensure_loaded()` away entirely while the setting is off, same as every
/// other call site that checks this (`apply_folder_diff`, `SearchWorker`).
#[tauri::command]
fn get_morph_analysis_active(kiwi: tauri::State<KiwiHandle>) -> bool {
    Config::load(Some(&settings_path()))
        .map(|config| config.enable_morphological_analysis)
        .unwrap_or(false)
        && kiwi.ensure_loaded()
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

/// Activates the app, making it the frontmost app. Called from the "about"
/// tray menu handler, after the tray menu has already closed - this app
/// runs with `ActivationPolicy::Accessory` (see `run()`'s `.setup()`), so it
/// never becomes the active app on its own, and the About dialog would
/// otherwise open behind whatever app currently has focus instead of coming
/// to the front.
/// Deliberately NOT called any earlier (e.g. on the tray icon's
/// right-mouse-down, before the menu opens) - activating the app while the
/// tray's context menu is opening/open makes macOS cancel that menu's
/// tracking session immediately, so the whole menu flashes and closes
/// instead of staying open.
#[cfg(target_os = "macos")]
fn activate_app() {
    let mtm = objc2::MainThreadMarker::new().expect("menu events are handled on the main thread");
    #[allow(deprecated)] // `NSApp.activate` (macOS 14+) has no equivalent for older macOS versions
    objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
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
    // Of the FULL-tier documents, how many actually got Kiwi's morphological
    // analysis vs. bigram-only (Kiwi unavailable, or `enable_morphological_analysis`
    // off - `core::config::Config`, `src-tauri`'s `KiwiActor`).
    let kiwi_analyzed = SearchRepository::count_kiwi_analyzed(&db.conn).map_err(|e| e.to_string())?;
    let last_indexed = DocumentRepository::last_indexed_at(&db.conn).map_err(|e| e.to_string())?;
    let size = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);

    Ok(format!(
        "Total: {} documents\n  Full text indexed: {full}\n    (Morphologically analyzed: {kiwi_analyzed})\n  Metadata only: {meta}\nDatabase size: {}\nLast indexed: {}",
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
    kiwi: KiwiHandle,
) {
    thread::spawn(move || {
        if let Err(e) = run_index_worker(
            db_path,
            settings_path,
            config,
            reset_rx,
            on_settings_reload,
            progress,
            kiwi,
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
    kiwi: KiwiHandle,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = Db::open(&db_path)?;
    let extractors = default_extractors();
    let bigram = BigramTokenizer;
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
        &kiwi,
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
                    &kiwi,
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
                    // The biggest single deletion this app ever does - always
                    // worth reclaiming (`Db::reclaim_space`'s doc comment).
                    if let Err(e) = db.reclaim_space() {
                        eprintln!("Failed to reclaim disk space: {e}");
                    }
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
                        &kiwi,
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
                // `config` is the live, settings-reloaded value, so flipping
                // `enable_morphological_analysis` off/on and saving takes
                // effect on the very next file change - same as
                // `apply_folder_diff`. `&&` short-circuits `ensure_loaded()`
                // (Kiwi's load attempt) away entirely while it's off.
                let kiwi_available =
                    config.enable_morphological_analysis && kiwi.ensure_loaded();
                let pipeline = IndexPipeline {
                    conn: &db.conn,
                    config: &config,
                    extractors: &extractors,
                    bigram: &bigram,
                    kiwi: if kiwi_available {
                        Some(&kiwi as &dyn Tokenizer)
                    } else {
                        None
                    },
                };
                let mut removed_anything = false;
                for (path, result) in queue::drain(&pipeline, events) {
                    match result {
                        Ok(queue::WatchOutcome::Removed) => removed_anything = true,
                        Err(e) => eprintln!("{}: index error: {e}", path.display()),
                        Ok(_) => {}
                    }
                }
                if removed_anything {
                    if let Err(e) = db.reclaim_space() {
                        eprintln!("Failed to reclaim disk space: {e}");
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
/// removed ones stop being watched.
///
/// Every already-indexed path outside all of `config.watched_folders` is
/// purged (`DocumentRepository::prune_paths_outside_watched`), unconditionally
/// on every call rather than only for folders this call's `added`/`removed`
/// diff notices. That diff is relative to this process's own in-memory
/// `current`, which starts empty on every fresh process (`run_index_worker`),
/// so a folder removed from `settings.json` while the app wasn't running at
/// all was never in `current` to begin with, and is neither "added" nor
/// "removed" from the diff's point of view; it would otherwise never be
/// noticed. Reconciling against the whole current `watched_folders` list
/// directly, instead of relying on that diff, catches both this
/// removed-while-closed case and an ordinary live removal with one
/// mechanism. No "does it still exist on disk" check here - the folder is
/// gone from *configuration*, regardless of whether it's still physically
/// present (contrast `prune_missing_paths_under` below).
///
/// Every scanned folder (not just a genuinely new one - see below) is also
/// reconciled against the filesystem via `DocumentRepository::
/// prune_missing_paths_under`, removing any already-indexed path that's
/// gone now. This is what catches files/folders deleted while the app
/// wasn't running at all: `current` always starts empty for a fresh process
/// (`run_index_worker`), so every folder in `watched_folders` is "added"
/// again on every app start, not just the first time it's configured -
/// without this, a deletion that happened during that downtime would never
/// surface (no live `notify` event to catch it, and the scan itself only
/// adds/updates files it currently finds, never removes ones it doesn't).
///
/// Calls `db.reclaim_space()` once at the end if any of the above actually
/// deleted rows - SQLite doesn't shrink its file on `DELETE` alone (see that
/// method's doc comment), so without this the DB file would stay at its
/// largest-ever size even after a folder full of documents is removed.
///
/// Triggers `kiwi`'s (shared, actor-owned) load attempt the first time any
/// folder is ever added *and* `config.enable_morphological_analysis` is on,
/// so an app with nothing configured yet - or with the setting left at its
/// default (off) - doesn't pay Kiwi's memory cost for nothing (`KiwiActor`'s
/// doc comment has the full reasoning on why this is a handle to a shared
/// instance rather than one owned locally).
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
    kiwi: &KiwiHandle,
    watcher: &mut FileWatcher,
    current: &mut Vec<PathBuf>,
    config: &Config,
    progress: &IndexProgressState,
) {
    let desired = &config.watched_folders;
    let added: Vec<&PathBuf> = desired.iter().filter(|f| !current.contains(f)).collect();
    let removed: Vec<&PathBuf> = current.iter().filter(|f| !desired.contains(f)).collect();

    // Tracks whether this call actually deleted any rows, so `db.reclaim_space()`
    // (see its own doc comment - SQLite doesn't shrink its file on `DELETE` alone)
    // only runs when there's something to reclaim, not on every settings-reload/
    // startup tick regardless.
    let mut removed_anything = false;

    // Unconditional, ahead of the added/removed-diff early return below - see
    // this function's doc comment for why relying on that diff alone misses
    // a folder removed while the app wasn't running at all.
    let canonical_desired: Vec<PathBuf> = desired.iter().map(|f| canonical_path(f)).collect();
    match DocumentRepository::prune_paths_outside_watched(&db.conn, &canonical_desired) {
        Ok(pruned) if !pruned.is_empty() => {
            eprintln!(
                "Removed {} document(s) no longer under any watched folder",
                pruned.len()
            );
            removed_anything = true;
        }
        Ok(_) => {}
        Err(e) => eprintln!("Failed to prune documents outside watched folders: {e}"),
    }

    if added.is_empty() && removed.is_empty() {
        if removed_anything {
            if let Err(e) = db.reclaim_space() {
                eprintln!("Failed to reclaim disk space: {e}");
            }
        }
        return;
    }

    let kiwi_available =
        !added.is_empty() && config.enable_morphological_analysis && kiwi.ensure_loaded();

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config,
        extractors,
        bigram,
        kiwi: if kiwi_available {
            Some(kiwi as &dyn Tokenizer)
        } else {
            None
        },
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
        // Catches deletions that happened while the app wasn't running (no
        // live `notify` event for `queue::handle_path` to react to) - the
        // scan above only ever adds/updates files it currently finds, so a
        // file gone since the last run wouldn't otherwise be noticed until
        // it's edited/replaced. Every watched folder goes through here on
        // every app start (`current` always starts empty for a fresh
        // process, so nothing is ever *not* "added"), not just first-time
        // folder adds.
        match DocumentRepository::prune_missing_paths_under(
            &db.conn,
            &canonical_path(folder).to_string_lossy(),
        ) {
            Ok(pruned) if !pruned.is_empty() => {
                eprintln!(
                    "Removed {} document(s) under {} no longer on disk",
                    pruned.len(),
                    folder.display()
                );
                removed_anything = true;
            }
            Ok(_) => {}
            Err(e) => eprintln!("Failed to prune missing paths under {}: {e}", folder.display()),
        }
        if let Err(e) = watcher.watch(folder) {
            eprintln!("Failed to watch {}: {e}", folder.display());
        }
    }
    *progress.lock().unwrap() = None; // Idle again - the whole batch is done.
    for folder in removed {
        // The DB purge for a no-longer-watched folder already happened above
        // (`prune_paths_outside_watched`, unconditionally) - this is just
        // live `notify` cleanup.
        if let Err(e) = watcher.unwatch(folder) {
            eprintln!("Failed to unwatch {}: {e}", folder.display());
        }
    }

    if removed_anything {
        if let Err(e) = db.reclaim_space() {
            eprintln!("Failed to reclaim disk space: {e}");
        }
    }

    *current = desired.clone();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
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
    // Shared by `SearchWorker` and the index/watch worker (spawned below) so
    // the two don't each load their own copy of Kiwi's model - see
    // `KiwiActor`'s doc comment.
    let kiwi = KiwiActor::spawn();
    let worker = SearchWorker::spawn(db_path.clone(), kiwi.clone());
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
        .manage(kiwi.clone())
        .invoke_handler(tauri::generate_handler![
            search,
            open_path,
            open_parent_folder,
            open_settings_file,
            get_theme,
            get_result_limit,
            get_search_debounce_ms,
            get_morph_analysis_active,
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
                kiwi,
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
            // A plain `MenuItem` (handled below in `on_menu_event`), not
            // `PredefinedMenuItem::about` - a `PredefinedMenuItem` action
            // runs natively and never reaches `on_menu_event`, but the
            // dialog shown here needs the app activated first to reliably
            // appear on top (see `activate_app()`), which only fires
            // cleanly once selection has already closed the tray menu.
            let about_item = MenuItem::with_id(app, "about", "About", true, None::<&str>)?;
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
                    "about" => {
                        // Activated here, after the tray menu selection has
                        // already closed the menu - see `activate_app()` for
                        // why it must not happen any earlier.
                        #[cfg(target_os = "macos")]
                        activate_app();
                        let text = format!(
                            "Version {}\nDeveloped by Yonghee Yu",
                            env!("CARGO_PKG_VERSION")
                        );
                        app.dialog()
                            .message(text)
                            .title("About KnowDesk")
                            .kind(MessageDialogKind::Info)
                            .show(|_| {});
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

        let worker = SearchWorker::spawn(db_path, KiwiActor::spawn());
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
            KiwiActor::spawn(),
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
            KiwiActor::spawn(),
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
    fn compute_stats_reports_kiwi_analyzed_count_among_full_text_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        {
            let db = Db::open(&db_path).unwrap();
            for id in ["kiwi1", "bigram_only"] {
                DocumentRepository::upsert_document(
                    &db.conn,
                    &DocumentRecord {
                        document_id: id.to_string(),
                        file_size: 10,
                        text_bytes: 5,
                        index_tier: IndexTier::Full,
                    },
                )
                .unwrap();
            }
            SearchRepository::index_content(&db.conn, "kiwi1", "본문", "본문", "본문").unwrap();
            // Kiwi unavailable/off - `morph_kiwi` left empty, same as
            // `extract_and_index`'s `unwrap_or_default()`.
            SearchRepository::index_content(&db.conn, "bigram_only", "본문", "본문", "").unwrap();
        }

        let stats = compute_stats(&db_path).unwrap();
        assert!(
            stats.contains("Full text indexed: 2\n    (Morphologically analyzed: 1)"),
            "{stats}"
        );
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
        let kiwi = KiwiActor::spawn();
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
            &kiwi,
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

    /// Reported bug: deleting an indexed file (or a whole folder) while the
    /// app isn't running at all left it stuck in the DB after the next
    /// launch - there's no live `notify` event to catch a deletion that
    /// happens during that downtime, and the initial scan alone only ever
    /// adds/updates files it currently finds, never removing ones it
    /// doesn't. Simulates a restart by calling `apply_folder_diff` a second
    /// time with a fresh, empty `current` (exactly what `run_index_worker`
    /// starts with on every process start) - the watched folder becomes
    /// "added" again just like on a real relaunch.
    #[test]
    fn apply_folder_diff_prunes_files_deleted_while_the_app_was_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("kept.txt");
        let deleted = dir.path().join("deleted.txt");
        std::fs::write(&kept, "채권 발행").unwrap();
        std::fs::write(&deleted, "이사회 결의").unwrap();

        let db = Db::open_in_memory().unwrap();
        let extractors = default_extractors();
        let bigram = BigramTokenizer;
        let kiwi = KiwiActor::spawn();
        let config = Config {
            watched_folders: vec![dir.path().to_path_buf()],
            ..Config::default()
        };
        let progress: IndexProgressState = Arc::new(Mutex::new(None));

        // First "run": both files get indexed.
        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &config,
            &progress,
        );
        let tiers = DocumentRepository::count_by_tier(&db.conn).unwrap();
        let full = tiers.iter().find(|(t, _)| t == "FULL").map_or(0, |(_, c)| *c);
        assert_eq!(full, 2, "both files should be indexed before the app 'restarts'");

        // The app is closed (no watcher running) and the user deletes one file.
        std::fs::remove_file(&deleted).unwrap();

        // Second "run": fresh `current`, like `run_index_worker` starts with
        // on every process start - the same folder is "added" all over again.
        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &config,
            &progress,
        );

        let tiers = DocumentRepository::count_by_tier(&db.conn).unwrap();
        let full = tiers.iter().find(|(t, _)| t == "FULL").map_or(0, |(_, c)| *c);
        assert_eq!(
            full, 1,
            "the file deleted while the app wasn't running must be pruned on the next scan"
        );

        let path_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
            .unwrap();
        assert_eq!(path_count, 1);
    }

    /// Removing a folder from `watched_folders` (hand-editing `settings.json`)
    /// must purge everything indexed from it, not just stop watching it -
    /// the file itself is left untouched on disk throughout, to pin that
    /// this is driven purely by the folder dropping out of *configuration*,
    /// not by anything happening on the filesystem.
    #[test]
    fn apply_folder_diff_purges_documents_when_a_folder_is_removed_from_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("규정.txt"), "채권 발행 절차").unwrap();

        let db = Db::open_in_memory().unwrap();
        let extractors = default_extractors();
        let bigram = BigramTokenizer;
        let kiwi = KiwiActor::spawn();
        let progress: IndexProgressState = Arc::new(Mutex::new(None));

        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        let watching_config = Config {
            watched_folders: vec![dir.path().to_path_buf()],
            ..Config::default()
        };
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &watching_config,
            &progress,
        );
        let tiers = DocumentRepository::count_by_tier(&db.conn).unwrap();
        let full = tiers.iter().find(|(t, _)| t == "FULL").map_or(0, |(_, c)| *c);
        assert_eq!(full, 1, "must be indexed before the folder is removed");

        // The folder is dropped from `watched_folders` - the file on disk is
        // never touched.
        let unwatched_config = Config {
            watched_folders: vec![],
            ..Config::default()
        };
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &unwatched_config,
            &progress,
        );

        assert!(
            DocumentRepository::count_by_tier(&db.conn).unwrap().is_empty(),
            "documents from a folder removed from watched_folders must be purged"
        );
        let path_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
            .unwrap();
        assert_eq!(path_count, 0);
        assert!(
            dir.path().join("규정.txt").exists(),
            "the file itself must be untouched on disk - only the index entry is removed"
        );
    }

    /// End-to-end check that removing a watched folder doesn't just delete
    /// DB rows but actually shrinks the `.db` file on disk - SQLite doesn't
    /// do this on `DELETE` alone (`Db::reclaim_space`'s doc comment), so
    /// this pins that `apply_folder_diff` actually calls it once it detects
    /// a purge happened, not just that the purge itself is correct (already
    /// covered by the row-count assertions above).
    #[test]
    fn apply_folder_diff_shrinks_the_db_file_after_removing_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..300 {
            std::fs::write(
                dir.path().join(format!("문서_{i}.txt")),
                "채권 발행 절차를 규정한다 ".repeat(500),
            )
            .unwrap();
        }

        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("test.db");
        let db = Db::open(&db_path).unwrap();
        let extractors = default_extractors();
        let bigram = BigramTokenizer;
        let kiwi = KiwiActor::spawn();
        let progress: IndexProgressState = Arc::new(Mutex::new(None));

        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        let watching_config = Config {
            watched_folders: vec![dir.path().to_path_buf()],
            ..Config::default()
        };
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &watching_config,
            &progress,
        );
        // WAL mode (`Db::open`) keeps recent writes in a `-wal` sidecar file
        // until checkpointed - measuring just the main file here would be
        // comparing against whatever fraction of the 300 documents happened
        // to already be checkpointed, not the true total. A manual
        // checkpoint (outside of `reclaim_space`, which isn't called yet at
        // this point) makes this a fair baseline.
        db.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let size_while_watched = std::fs::metadata(&db_path).unwrap().len();

        let unwatched_config = Config {
            watched_folders: vec![],
            ..Config::default()
        };
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &unwatched_config,
            &progress,
        );
        let size_after_removal = std::fs::metadata(&db_path).unwrap().len();

        assert!(
            size_after_removal < size_while_watched,
            "expected the .db file to shrink after the folder was removed: \
             while_watched={size_while_watched}, after_removal={size_after_removal}"
        );
    }

    /// Reported bug: removing a folder from `watched_folders` while the app
    /// is running purges it correctly (previous test), but doing the same
    /// hand-edit while the app is *closed*, then starting it, left the
    /// documents stuck - `current` starts empty on a fresh process, so the
    /// removed folder was never in it to begin with, and the plain
    /// added/removed diff never notices a folder that's simply absent from
    /// both sides. Simulates this by skipping the "still watching" first
    /// call entirely - `apply_folder_diff` only ever sees the config with
    /// the folder already gone, exactly like a real restart after a
    /// settings.json hand-edit made while the app wasn't running.
    #[test]
    fn apply_folder_diff_purges_a_folder_removed_from_config_while_the_app_was_closed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("규정.txt"), "채권 발행 절차").unwrap();

        let db = Db::open_in_memory().unwrap();
        let extractors = default_extractors();
        let bigram = BigramTokenizer;
        let kiwi = KiwiActor::spawn();
        let progress: IndexProgressState = Arc::new(Mutex::new(None));

        // Directly seed the DB as if a *previous* run had indexed this
        // folder - no `apply_folder_diff` call with it in `watched_folders`
        // happens in this test at all, matching a process that starts fresh
        // after the folder was already removed from settings.json.
        let pipeline = IndexPipeline {
            conn: &db.conn,
            config: &Config::default(),
            extractors: &extractors,
            bigram: &bigram,
            kiwi: None,
        };
        pipeline.index_directory(dir.path()).unwrap();
        let tiers = DocumentRepository::count_by_tier(&db.conn).unwrap();
        let full = tiers.iter().find(|(t, _)| t == "FULL").map_or(0, |(_, c)| *c);
        assert_eq!(full, 1, "premise: the folder must already be indexed");

        // "Startup": fresh `current`, and `watched_folders` already excludes
        // the folder - `added`/`removed` are both empty (the folder is on
        // neither side), which is exactly the case the old diff-only logic
        // couldn't catch.
        let mut watcher = FileWatcher::new::<PathBuf>(&[], Duration::from_millis(3000)).unwrap();
        let mut watched: Vec<PathBuf> = Vec::new();
        let config = Config {
            watched_folders: vec![],
            ..Config::default()
        };
        apply_folder_diff(
            &db,
            &extractors,
            &bigram,
            &kiwi,
            &mut watcher,
            &mut watched,
            &config,
            &progress,
        );

        assert!(
            DocumentRepository::count_by_tier(&db.conn).unwrap().is_empty(),
            "a folder removed from watched_folders while the app was closed must still be purged on the next start"
        );
    }
}
