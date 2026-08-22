// Thin IPC bindings only - every command delegates to the `knowdesk-core` crate
// (`CLAUDE.md`: "core는 Tauri를 절대 참조하지 않는다. 모든 OS 통합은 src-tauri로 격리한다").
// `open_path`/`open_parent_folder` are the exception (native opener, no equivalent in `core`).

use knowdesk_core::db::Db;
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
use tauri::AppHandle;
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
/// (`README.md`'s `--db`-less CLI behavior is cwd-relative, which isn't
/// appropriate for a GUI app launched from anywhere).
fn db_path() -> PathBuf {
    if let Ok(path) = std::env::var("KNOWDESK_DB_PATH") {
        return PathBuf::from(path);
    }
    let dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("KnowDesk");
    dir.join("knowdesk.db")
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db_path = db_path();
    if let Some(parent) = db_path.parent() {
        // Best-effort - `Db::open` below fails loudly if this didn't work.
        let _ = std::fs::create_dir_all(parent);
    }
    let worker = SearchWorker::spawn(db_path);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(worker)
        .invoke_handler(tauri::generate_handler![
            search,
            open_path,
            open_parent_folder
        ])
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
