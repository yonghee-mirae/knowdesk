//! Integration verification for Phase B4's completion criteria: file creation/deletion is
//! reflected in real time, and the indexing pipeline's own file reads don't generate new
//! events that trigger an infinite re-indexing loop.

use knowdesk_core::config::Config;
use knowdesk_core::db::Db;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::index::queue::{self, WatchOutcome};
use knowdesk_core::index::watcher::FileWatcher;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{SearchMode, SearchRequest, SearchService};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(200);
// Max time to wait for an event. Must be generous compared to the debounce.
const WAIT: Duration = Duration::from_secs(5);

#[test]
fn same_file_via_different_path_strings_is_treated_as_one_document() {
    // A real bug that occurred: `cli watch`'s initial full scan uses the path exactly as
    // given by the user (e.g. "./samples/x.txt"), but afterward when `notify` reports a
    // change, it gives an event with an absolute path prefixed by the current working
    // directory ("/current/directory/./samples/x.txt"). Without canonicalize, these two
    // are treated as different files, so the same file gets indexed as two separate
    // documents, and even after editing the content, the old content stayed permanently
    // in search results. Here, to reproduce the same problem without changing cwd, we
    // point at the same file using a different string with a "./" inserted in the middle
    // (matching the shape of the paths in the real bug).
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("보고서.txt");
    std::fs::write(&file_path, "GDP 성장률은 3.2%였다.").unwrap();

    let db = Db::open_in_memory().unwrap();
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

    pipeline.index_file(&file_path).unwrap();

    std::fs::write(&file_path, "GDP 성장률은 5.2%였다.").unwrap();
    let differently_written_path = dir.path().join(".").join(file_path.file_name().unwrap());
    // `PathBuf`'s `==` compares component-by-component and ignores the "." in the middle,
    // so it reports them as equal — the problem is that these string representations differ
    // when building the DB key via `path.to_string_lossy()` (the actual cause of the bug).
    assert_ne!(
        file_path.to_string_lossy(),
        differently_written_path.to_string_lossy(),
        "test premise broken: the string representations of the two paths must not be equal to begin with"
    );
    pipeline.index_file(&differently_written_path).unwrap();

    let path_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        path_count, 1,
        "the same file was indexed split across two paths"
    );

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let old_content = search
        .search(&SearchRequest {
            query: "3.2%".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        old_content.hits.is_empty(),
        "old content is still found by search: {:?}",
        old_content.hits
    );

    let new_content = search
        .search(&SearchRequest {
            query: "5.2%".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert_eq!(new_content.hits.len(), 1, "hits: {:?}", new_content.hits);
}

#[test]
fn reindexing_an_edited_file_cleans_up_the_previous_content_hash() {
    // `document_id` is a content hash (`core::scan::hash`), so editing an
    // already-indexed file's content between app runs (no live `notify`
    // event to see it as a change - the app wasn't watching, or wasn't even
    // running) makes the next scan of that path compute a brand new
    // `document_id` and re-extract it as if it were a different document
    // entirely. Confirms the *previous* hash's `documents`/`content_fts`/
    // `document_bodies` rows don't survive as permanent orphans -
    // `DocumentRepository::upsert_path`'s job, not exercised by
    // `same_file_via_different_path_strings_is_treated_as_one_document`
    // above (which only asserts on search results, not raw row counts).
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("보고서.txt");
    std::fs::write(&file_path, "GDP 성장률은 3.2%였다.").unwrap();

    let db = Db::open_in_memory().unwrap();
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
    pipeline.index_file(&file_path).unwrap();

    let document_count = || -> i64 {
        db.conn
            .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(document_count(), 1, "one document after the first index");

    // Edited "while the app wasn't running" - just a plain file write, no
    // watcher/notify event involved - then the same path is scanned again,
    // exactly like `apply_folder_diff`'s startup scan re-walking an
    // already-watched folder.
    std::fs::write(&file_path, "GDP 성장률은 5.2%였다.").unwrap();
    pipeline.index_file(&file_path).unwrap();

    assert_eq!(
        document_count(),
        1,
        "the old content-hash document must be cleaned up, not left as a permanent orphan"
    );
    let content_fts_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM content_fts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(content_fts_count, 1);
    let document_bodies_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM document_bodies", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(document_bodies_count, 1);
}

#[test]
fn watch_indexes_new_file_and_removes_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
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

    let watcher = FileWatcher::new(&[dir.path()], DEBOUNCE).unwrap();

    let file_path = dir.path().join("규정.txt");
    std::fs::write(&file_path, "채권 발행 절차").unwrap();

    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive a creation event");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "events: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Indexed(_))));

    // Confirm the indexed content is actually searchable.
    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);

    // The indexing pipeline just read this file (hash computation, text extraction) — that
    // read itself must not generate a new event that triggers infinite re-indexing (a real
    // bug that occurred, see `watcher.rs`). Even after waiting long enough, there should be
    // no further events.
    assert!(
        watcher.recv_timeout(WAIT).is_none(),
        "it looks like the indexing pipeline's own read generated an event, causing a re-indexing loop"
    );

    // Deleting the file should remove it from the index.
    std::fs::remove_file(&file_path).unwrap();
    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive a deletion event");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "events: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Removed)));

    let result = search
        .search(&SearchRequest {
            query: "채권".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        result.hits.is_empty(),
        "still found by search after deletion: {:?}",
        result.hits
    );
}

#[test]
fn watch_removes_every_document_when_a_whole_subfolder_is_deleted() {
    // Reported bug: deleting an entire indexed folder (not just one file
    // inside it) left its documents stuck in the DB. Root cause was
    // `canonical_path` only walking up one ancestor level to reconstruct a
    // deleted path's canonical form, plus `remove_path` only ever matching
    // one exact `paths.path` value - neither handles a file whose *parent*
    // is also gone, which is exactly what happens when the containing
    // folder is deleted as a whole rather than emptied file-by-file.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
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

    let watcher = FileWatcher::new(&[dir.path()], DEBOUNCE).unwrap();

    let sub_dir = dir.path().join("sub");
    std::fs::create_dir(&sub_dir).unwrap();
    std::fs::write(sub_dir.join("규정.txt"), "채권 발행 절차").unwrap();
    std::fs::write(sub_dir.join("결의.txt"), "이사회 결의").unwrap();

    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive creation events");
    let outcomes = queue::drain(&pipeline, events);
    // The batch may also include a `Create` event for `sub` itself (ignored,
    // directories aren't indexing targets - `handle_path`'s `is_dir()` check)
    // alongside the two files' - not asserted on directly since whether that
    // shows up at all is OS/timing-dependent, unlike the two files' outcomes.
    let indexed = outcomes
        .iter()
        .filter(|(_, outcome)| matches!(outcome, Ok(WatchOutcome::Indexed(_))))
        .count();
    assert_eq!(indexed, 2, "events: {:?}", outcomes);

    let path_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
        .unwrap();
    assert_eq!(path_count, 2, "both files should be indexed before deletion");

    // Delete the whole subfolder at once, not file-by-file - both files' own
    // parent directory is gone too, not just the files themselves.
    std::fs::remove_dir_all(&sub_dir).unwrap();

    // A generous number of drain passes: depending on how the OS coalesces
    // the bulk deletion into `notify` events (per-file vs. one event for the
    // folder itself), it may take more than one debounced batch to fully
    // settle.
    for _ in 0..10 {
        let Some(events) = watcher.recv_timeout(WAIT) else {
            break;
        };
        queue::drain(&pipeline, events);
    }

    let path_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM paths", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        path_count, 0,
        "documents from a wholly-deleted folder must not stay stuck in the DB"
    );

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        result.hits.is_empty(),
        "still found by search after the whole folder was deleted: {:?}",
        result.hits
    );
}

#[test]
fn watch_multiple_roots_on_a_single_watcher() {
    // `src-tauri`'s index worker watches every configured folder on one
    // `FileWatcher` (one thread, one `KiwiTokenizer`) rather than one per
    // folder (`core/src/index/watcher.rs`'s doc comment on `new`) - this pins
    // that a single watcher instance actually picks up changes from more
    // than one of its watched roots, not just the first.
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
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

    let watcher = FileWatcher::new(&[dir_a.path(), dir_b.path()], DEBOUNCE).unwrap();

    std::fs::write(dir_a.path().join("규정.txt"), "채권 발행 절차").unwrap();
    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive a creation event from the first root");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "events: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Indexed(_))));

    std::fs::write(dir_b.path().join("결의.txt"), "이사회 결의").unwrap();
    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive a creation event from the second root");
    let outcomes = queue::drain(&pipeline, events);
    assert_eq!(outcomes.len(), 1, "events: {:?}", outcomes);
    assert!(matches!(outcomes[0].1, Ok(WatchOutcome::Indexed(_))));
}

#[test]
fn watch_and_unwatch_change_the_live_root_set() {
    // "설정 적용"/Reload (`src-tauri`'s index worker) adds/removes roots on an
    // already-running `FileWatcher` instead of recreating it - pins that
    // `watch`/`unwatch` actually take effect live.
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();

    let mut watcher = FileWatcher::new(&[dir_a.path()], DEBOUNCE).unwrap();

    // `dir_b` isn't watched yet - a change there produces nothing.
    std::fs::write(dir_b.path().join("결의.txt"), "이사회 결의").unwrap();
    assert!(
        watcher.recv_timeout(WAIT).is_none(),
        "an unwatched root's change must not produce an event"
    );

    // Watching it live picks up subsequent changes.
    watcher.watch(dir_b.path()).unwrap();
    std::fs::write(dir_b.path().join("결의2.txt"), "예산 승인").unwrap();
    assert!(
        watcher.recv_timeout(WAIT).is_some(),
        "did not receive an event from a root added live via watch()"
    );

    // Unwatching `dir_a` live stops further events from it, while `dir_b`
    // (still watched) keeps working.
    watcher.unwatch(dir_a.path()).unwrap();
    std::fs::write(dir_a.path().join("규정.txt"), "채권 발행 절차").unwrap();
    assert!(
        watcher.recv_timeout(WAIT).is_none(),
        "a root removed live via unwatch() must not produce further events"
    );
    std::fs::write(dir_b.path().join("결의3.txt"), "또 다른 결의").unwrap();
    assert!(
        watcher.recv_timeout(WAIT).is_some(),
        "an untouched, still-watched root must keep producing events"
    );
}

#[test]
fn set_debounce_takes_effect_immediately() {
    // Live-reloading `settings.json`'s `file_watch_debounce_ms` (`src-tauri`'s
    // index worker) mutates the field in place rather than recreating the
    // watcher - pins that a change actually applies to the very next settle,
    // not just on some later watcher recreation.
    let dir = tempfile::tempdir().unwrap();
    // Starts with a debounce far longer than this test should ever have to
    // wait, so a passing result can only be explained by `set_debounce`
    // actually taking effect - not by coincidentally finishing before the
    // original value's settle window would have elapsed anyway.
    let mut watcher = FileWatcher::new(&[dir.path()], Duration::from_secs(5)).unwrap();
    watcher.set_debounce(Duration::from_millis(50));

    std::fs::write(dir.path().join("규정.txt"), "채권 발행 절차").unwrap();

    let start = std::time::Instant::now();
    assert!(
        watcher.recv_timeout(WAIT).is_some(),
        "expected a change event"
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "set_debounce should have taken effect immediately, took {:?}",
        start.elapsed()
    );
}

#[test]
fn watch_ignores_short_lived_temp_file_from_office_style_save() {
    // Simulates the temp file (`~$filename`) that Office creates on save and soon deletes.
    // If it disappears within the debounce window, there should be no indexing attempt at all.
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_in_memory().unwrap();
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

    let watcher = FileWatcher::new(&[dir.path()], DEBOUNCE).unwrap();

    // Canonicalize before building the expected paths below: on macOS, `notify`'s
    // FSEvents backend reports paths resolved through `/var` -> `/private/var` (a
    // system symlink), while `dir.path()` itself is not resolved. Comparing the
    // unresolved form against event paths from `queue::drain` below would then never
    // match, even though the file was correctly detected and indexed. This is a no-op
    // on platforms without such a symlink (e.g. Linux).
    let dir_path = dir.path().canonicalize().unwrap();
    let temp_path = dir_path.join("~$규정.txt");
    let real_path = dir_path.join("규정.txt");
    std::fs::write(&temp_path, "임시").unwrap();
    std::fs::remove_file(&temp_path).unwrap();
    std::fs::write(&real_path, "본 문서는 채권 발행 절차를 규정한다.").unwrap();

    let events = watcher
        .recv_timeout(WAIT)
        .expect("did not receive an event");
    let outcomes = queue::drain(&pipeline, events);

    let real_outcome = outcomes
        .iter()
        .find(|(path, _)| path == &real_path)
        .expect("no event for the real file");
    assert!(matches!(real_outcome.1, Ok(WatchOutcome::Indexed(_))));

    if let Some((_, temp_outcome)) = outcomes.iter().find(|(path, _)| path == &temp_path) {
        // Even if a temp file event also arrived, it must be ignored since the file was
        // already gone before indexing — the extension itself is supported (.txt), so if
        // the file had still existed, it would have been indexed rather than SKIPped.
        assert!(
            matches!(temp_outcome, Ok(WatchOutcome::Ignored)),
            "temp file handling result: {:?}",
            temp_outcome
        );
    }
}
