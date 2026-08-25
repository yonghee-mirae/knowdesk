//! Parallel indexing - **`kdfind`-only**. `knowdesk-cli`'s subcommands and the
//! GUI app keep using `core::index::pipeline::IndexPipeline`'s single-threaded
//! path unchanged (`docs/06_Development_Roadmap.md` B4 — that path is
//! deliberately left unthrottled-but-single-threaded, not sped up further).
//! `kdfind` is a one-shot tool with no idle-CPU budget to protect, so it's
//! worth spending every core to finish faster.
//!
//! This reuses `core`'s already-`pub`, stateless building blocks directly
//! (`ContentExtractor`, `Tokenizer`, `scan::hash`, `scan::filter`,
//! `db::documents`/`db::search_repo`/`db::store`) instead of
//! `IndexPipeline::index_file` itself, because `rusqlite::Connection` is
//! `Send` but not `Sync` — `IndexPipeline` holds a bare `&Connection`, so its
//! methods can never be called from more than one thread at a time no matter
//! what wraps them. None of `core`/`src-tauri` needed to change for this.

use knowdesk_core::config::Config;
use knowdesk_core::db::documents::{DocumentRecord, DocumentRepository, IndexTier, PathRecord};
use knowdesk_core::db::search_repo::SearchRepository;
use knowdesk_core::db::store::{DocumentStore, SqliteDocumentStore};
use knowdesk_core::extract::{ContentExtractor, DocumentInfo};
use knowdesk_core::index::canonical_path;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::nlp::{join_tokens, Token, Tokenizer};
use knowdesk_core::scan::{filter, hash, walker};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, Default)]
pub struct IndexOutcome {
    pub full: u64,
    pub meta: u64,
    pub skip: u64,
}

/// Confines a `KiwiTokenizer` to one dedicated thread and lets any number of
/// worker threads request `tokenize`/`locate` calls through a channel instead
/// — `kiwi_rs::Kiwi` contains raw pointers and is not `Send`, so a value can
/// never move to (or be called from) another thread once built; this is the
/// same constraint `src-tauri`'s `KiwiActor`/`KiwiHandle` exists for,
/// reimplemented here so `cli` doesn't need to depend on `src-tauri` (nor vice
/// versa). `Clone`s share the same underlying actor thread - the standard
/// multi-producer/single-consumer pattern `mpsc` is named for.
#[derive(Clone)]
pub struct KiwiHandle {
    sender: mpsc::Sender<KiwiJob>,
}

enum KiwiJob {
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

impl KiwiHandle {
    /// Spawns the dedicated actor thread and builds the `KiwiTokenizer` *on
    /// that thread*, not before - since it isn't `Send`, an already-built
    /// instance could never be moved into the new thread's closure at all
    /// (confirmed by the compiler: `kiwi_rs::runtime::Kiwi` holds a `*mut
    /// c_void` and boxed callback trait objects, both `!Send`). Same reason
    /// `src-tauri`'s `KiwiActor::spawn` calls its own `load_kiwi()` from
    /// inside the spawned closure instead of passing in an existing value.
    /// Blocks until the load attempt finishes so the caller finds out
    /// immediately whether Kiwi is actually available (and can print exactly
    /// why not, same as before) rather than only on the first search.
    pub fn spawn(lib_path: PathBuf, model_dir: PathBuf) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel::<KiwiJob>();
        let (ready, ready_rx) = mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let kiwi = match KiwiTokenizer::new(lib_path, model_dir) {
                Ok(kiwi) => kiwi,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            let _ = ready.send(Ok(()));
            for job in receiver {
                match job {
                    KiwiJob::Tokenize { text, reply } => {
                        let _ = reply.send(kiwi.tokenize(&text));
                    }
                    KiwiJob::Locate { text, forms, reply } => {
                        let _ = reply.send(kiwi.locate(&text, &forms));
                    }
                }
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { sender }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("Kiwi actor thread exited unexpectedly".to_string()),
        }
    }
}

impl Tokenizer for KiwiHandle {
    fn tokenize(&self, text: &str) -> Vec<Token> {
        let (reply, reply_rx) = mpsc::channel();
        let job = KiwiJob::Tokenize {
            text: text.to_string(),
            reply,
        };
        if self.sender.send(job).is_err() {
            return Vec::new();
        }
        reply_rx.recv().unwrap_or_default()
    }

    fn locate(&self, text: &str, forms: &[String]) -> Option<(usize, usize)> {
        let (reply, reply_rx) = mpsc::channel();
        let job = KiwiJob::Locate {
            text: text.to_string(),
            forms: forms.to_vec(),
            reply,
        };
        if self.sender.send(job).is_err() {
            return None;
        }
        reply_rx.recv().ok().flatten()
    }
}

/// Scans and indexes every file under `root` using up to `threads` worker
/// threads, taking ownership of `conn` and handing it back once every file has
/// been processed (so the caller can search with it afterward). Extraction,
/// hashing, and bigram tokenization run fully in parallel across threads; two
/// things don't scale with thread count regardless, but stay correct:
///
/// - PDF extraction serializes internally no matter how many threads call it —
///   `pdfium-render` wraps every actual Pdfium call in its own mutex (its
///   README's "Multi-threading" section: Pdfium itself isn't thread-safe, so
///   the crate sequences all calls to avoid crashes, "no performance benefit"
///   by its own description).
/// - Kiwi tokenization (`kiwi`, if set) is funneled through its one dedicated
///   actor thread via `KiwiHandle`, same as `src-tauri`'s `KiwiActor`.
///
/// SQLite access is serialized through a `Mutex<Connection>` — `Connection` is
/// `Send` but not `Sync`, so this is the only way to let several threads write
/// to the same in-memory DB at all. Locked only for the brief read/write calls
/// themselves, never across a file's hashing/extraction/tokenization.
///
/// A per-file IO/DB error is logged and that file is counted as skipped rather
/// than aborting the whole run — deliberately different from
/// `IndexPipeline::index_directory`'s sequential path (which propagates the
/// first error via `?`), since one unreadable file shouldn't throw away work
/// already done by every other thread.
pub fn index_directory_parallel(
    root: &Path,
    config: &Config,
    extractors: &[Box<dyn ContentExtractor + Send + Sync>],
    kiwi: Option<KiwiHandle>,
    conn: Connection,
    threads: usize,
) -> (Connection, IndexOutcome) {
    let paths = walker::scan(root);
    let threads = threads.clamp(1, paths.len().max(1));

    let next = AtomicUsize::new(0);
    let full = AtomicU64::new(0);
    let meta = AtomicU64::new(0);
    let skip = AtomicU64::new(0);
    let conn_mutex = Mutex::new(conn);

    std::thread::scope(|scope| {
        for _ in 0..threads {
            let kiwi = kiwi.clone();
            let paths = &paths;
            let next = &next;
            let full = &full;
            let meta = &meta;
            let skip = &skip;
            let conn_mutex = &conn_mutex;
            scope.spawn(move || loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(idx) else {
                    break;
                };
                match index_one_file(path, config, extractors, kiwi.as_ref(), conn_mutex) {
                    IndexTier::Full => full.fetch_add(1, Ordering::Relaxed),
                    IndexTier::Meta => meta.fetch_add(1, Ordering::Relaxed),
                    IndexTier::Skip => skip.fetch_add(1, Ordering::Relaxed),
                };
            });
        }
    });

    let conn = conn_mutex
        .into_inner()
        .expect("no thread panicked while holding the lock");
    (
        conn,
        IndexOutcome {
            full: full.load(Ordering::Relaxed),
            meta: meta.load(Ordering::Relaxed),
            skip: skip.load(Ordering::Relaxed),
        },
    )
}

/// One file's worth of `IndexPipeline::index_file`/`extract_and_index`
/// (`core/src/index/pipeline.rs`), reimplemented here against a
/// `Mutex<Connection>` instead of a bare `&Connection` so it's safe to call
/// from several threads at once. Kept in lockstep with that logic by hand —
/// see this module's doc comment for why it can't just call the original
/// directly.
fn index_one_file(
    path: &Path,
    config: &Config,
    extractors: &[Box<dyn ContentExtractor + Send + Sync>],
    kiwi: Option<&KiwiHandle>,
    conn_mutex: &Mutex<Connection>,
) -> IndexTier {
    let path = canonical_path(path);
    let metadata = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to stat file, skipping");
            return IndexTier::Skip;
        }
    };
    let file_size = metadata.len();

    if filter::check(&path, file_size, config).is_some() {
        return IndexTier::Skip;
    }

    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let Some(extractor) = extractors.iter().find(|e| e.supports(&extension)) else {
        return IndexTier::Skip; // unsupported format
    };

    let document_id = match hash::hash_file(&path) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to hash file, skipping");
            return IndexTier::Skip;
        }
    };
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let modified_at = metadata.modified().ok().map(format_system_time);

    let existing_tier = {
        let conn = conn_mutex.lock().unwrap();
        DocumentRepository::get_tier(&conn, &document_id).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "failed to read existing tier");
            None
        })
    };

    let tier = existing_tier.unwrap_or_else(|| {
        extract_and_index(
            &document_id,
            &path,
            &extension,
            file_size,
            extractor.as_ref(),
            kiwi,
            conn_mutex,
        )
    });

    {
        let conn = conn_mutex.lock().unwrap();
        if let Err(e) = DocumentRepository::upsert_path(
            &conn,
            &PathRecord {
                path: path.to_string_lossy().to_string(),
                document_id,
                filename,
                extension,
                modified_at,
            },
        ) {
            tracing::warn!(path = %path.display(), error = %e, "failed to record path");
        }
    }

    tier
}

/// Same shape as `core::index::pipeline::IndexPipeline::extract_and_index` -
/// bigram always runs, Kiwi only if `kiwi` is set. Extraction/tokenization
/// happen without holding `conn_mutex` (the expensive, parallelizable part);
/// the lock is only taken for the write at the end.
fn extract_and_index(
    document_id: &str,
    path: &Path,
    extension: &str,
    file_size: u64,
    extractor: &(dyn ContentExtractor + Send + Sync),
    kiwi: Option<&KiwiHandle>,
    conn_mutex: &Mutex<Connection>,
) -> IndexTier {
    let document_info = DocumentInfo {
        path: path.to_path_buf(),
        extension: extension.to_string(),
    };

    match extractor.extract(&document_info) {
        Ok(result) => {
            let morph = join_tokens(&BigramTokenizer.tokenize(&result.body));
            let morph_kiwi = kiwi
                .map(|k| join_tokens(&k.tokenize(&result.body)))
                .unwrap_or_default();

            let conn = conn_mutex.lock().unwrap();
            if let Err(e) = DocumentRepository::upsert_document(
                &conn,
                &DocumentRecord {
                    document_id: document_id.to_string(),
                    file_size: file_size as i64,
                    text_bytes: result.body.len() as i64,
                    index_tier: IndexTier::Full,
                },
            ) {
                tracing::warn!(path = %path.display(), error = %e, "failed to record document");
            }
            if let Err(e) =
                (SqliteDocumentStore { conn: &conn }).put_body(document_id, &result.body)
            {
                tracing::warn!(path = %path.display(), error = %e, "failed to store body");
            }
            if let Err(e) = SearchRepository::index_content(
                &conn,
                document_id,
                &result.body,
                &morph,
                &morph_kiwi,
            ) {
                tracing::warn!(path = %path.display(), error = %e, "failed to index content");
            }
            IndexTier::Full
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "extraction failed, demoting to META");
            let conn = conn_mutex.lock().unwrap();
            if let Err(e) = DocumentRepository::upsert_document(
                &conn,
                &DocumentRecord {
                    document_id: document_id.to_string(),
                    file_size: file_size as i64,
                    text_bytes: 0,
                    index_tier: IndexTier::Meta,
                },
            ) {
                tracing::warn!(path = %path.display(), error = %e, "failed to record document");
            }
            IndexTier::Meta
        }
    }
}

/// Copy of `core::index::pipeline`'s private helper of the same name - not
/// `pub` there, so duplicated here rather than changing `core` for a
/// three-line function (`docs/13_CLI_Tool.md`: this module intentionally
/// touches nothing under `core/`).
fn format_system_time(t: std::time::SystemTime) -> String {
    time::OffsetDateTime::from(t)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowdesk_core::db::Db;
    use knowdesk_core::extract::txt::TxtExtractor;

    fn extractors() -> Vec<Box<dyn ContentExtractor + Send + Sync>> {
        vec![Box::new(TxtExtractor)]
    }

    #[test]
    fn indexes_every_file_across_multiple_threads() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..20 {
            std::fs::write(
                dir.path().join(format!("문서{i}.txt")),
                format!("채권 발행 {i}"),
            )
            .unwrap();
        }

        let db = Db::open_in_memory().unwrap();
        let config = Config::default();
        let (conn, outcome) =
            index_directory_parallel(dir.path(), &config, &extractors(), None, db.conn, 8);

        assert_eq!(outcome.full, 20);
        assert_eq!(outcome.meta, 0);
        assert_eq!(outcome.skip, 0);

        let counts = DocumentRepository::count_by_tier(&conn).unwrap();
        let full_count: i64 = counts
            .iter()
            .find(|(tier, _)| tier == "FULL")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(full_count, 20);
    }

    #[test]
    fn duplicate_content_across_files_collapses_to_one_document() {
        // Documents are deduped by content hash - several files with identical
        // bytes must still end up as exactly one `documents` row, even when
        // processed by different threads concurrently (the `get_tier` check
        // and the write are both individually locked, so this can't race into
        // duplicate rows - `upsert_document`/`index_content` are idempotent
        // upserts either way).
        let dir = tempfile::tempdir().unwrap();
        for i in 0..10 {
            std::fs::write(dir.path().join(format!("사본{i}.txt")), "같은 내용").unwrap();
        }

        let db = Db::open_in_memory().unwrap();
        let config = Config::default();
        let (conn, outcome) =
            index_directory_parallel(dir.path(), &config, &extractors(), None, db.conn, 8);

        assert_eq!(outcome.full, 10, "every path is still recorded");
        let counts = DocumentRepository::count_by_tier(&conn).unwrap();
        let full_count: i64 = counts
            .iter()
            .find(|(tier, _)| tier == "FULL")
            .map(|(_, n)| *n)
            .unwrap_or(0);
        assert_eq!(
            full_count, 1,
            "identical content must collapse to one document"
        );
    }

    #[test]
    fn unsupported_and_temp_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("문서.txt"), "채권").unwrap();
        std::fs::write(dir.path().join("압축.zip"), "not a real zip").unwrap();
        std::fs::write(dir.path().join("~$문서.txt"), "temp file").unwrap();

        let db = Db::open_in_memory().unwrap();
        let config = Config::default();
        let (_conn, outcome) =
            index_directory_parallel(dir.path(), &config, &extractors(), None, db.conn, 4);

        assert_eq!(outcome.full, 1);
        assert_eq!(outcome.skip, 2);
    }

    #[test]
    fn works_with_a_single_thread_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("문서.txt"), "채권 발행").unwrap();

        let db = Db::open_in_memory().unwrap();
        let config = Config::default();
        let (_conn, outcome) =
            index_directory_parallel(dir.path(), &config, &extractors(), None, db.conn, 1);

        assert_eq!(outcome.full, 1);
    }
}
