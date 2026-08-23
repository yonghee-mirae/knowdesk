pub mod documents;
pub mod migrate;
pub mod schema;
pub mod search_repo;
pub mod store;

use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

/// Index DB handle. Returned with migrations already applied.
pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // Without this, two connections opening the same not-yet-created DB file at
        // nearly the same moment (e.g. `src-tauri`'s `SearchWorker` and index worker
        // both starting up) can hit `SQLITE_BUSY` immediately instead of just waiting
        // for the other to finish its schema migration - confirmed in practice.
        // rusqlite's default busy handler is a no-op (fails immediately), so this has
        // to be set explicitly.
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate::run(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrate::run(&conn)?;
        Ok(Self { conn })
    }

    /// Reclaims disk space freed by row deletions - a deleted/unwatched file or folder
    /// (`db::documents::DocumentRepository`'s `remove_path`/`remove_paths_under`/
    /// `prune_missing_paths_under`/`prune_paths_outside_watched`/`reset_all`) frees rows,
    /// but SQLite never shrinks the database file on `DELETE` alone - the freed pages just
    /// sit in the file's internal freelist for reuse, not returned to the OS.
    ///
    /// A full `VACUUM`, not `PRAGMA incremental_vacuum` - despite that being the
    /// lighter-weight, purpose-built tool for exactly this. Confirmed empirically (not
    /// just per its docs) that it doesn't actually work here: `PRAGMA
    /// incremental_vacuum(N)` only ever reclaimed about *one* page per call in this
    /// environment regardless of how large N was, even with thousands of pages sitting
    /// in the freelist - so between that and `content_fts` below, a plain `VACUUM` is
    /// what's actually reliable, at the cost of being a full rebuild (needs roughly the
    /// DB's current size again in free disk space while it runs, and blocks other
    /// writers until it finishes).
    ///
    /// Also runs FTS5's `optimize` command on `content_fts` first: FTS5 doesn't remove
    /// a deleted row's data immediately either - `DELETE` just adds a tombstone marker,
    /// and the actual old segment data (often the largest chunk of a document - its
    /// full body text) isn't reclaimed until segments are merged, which `optimize` does
    /// explicitly. Skipping this and only running `VACUUM` still shrinks the file
    /// (VACUUM does at least drop `content_fts` rows that are gone by the time it
    /// copies the database into a new file) but measurably less than doing both -
    /// confirmed empirically as well, not left untested on the assumption docs are
    /// enough here after the `incremental_vacuum` surprise above.
    ///
    /// Not cheap - callers should only call this after an operation that's known to
    /// have actually deleted rows, not unconditionally.
    ///
    /// Followed by a WAL checkpoint (`Db::open` sets `journal_mode = WAL`) - confirmed
    /// empirically necessary (not just theoretical) in at least one real call path
    /// (`apply_folder_diff`): `PRAGMA page_count` correctly showed the rebuilt,
    /// tiny database right after `VACUUM` returned, yet the main `.db` file's size on
    /// disk stayed completely unchanged until this ran too.
    pub fn reclaim_space(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "INSERT INTO content_fts(content_fts) VALUES('optimize'); \
             VACUUM; \
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::documents::{DocumentRecord, DocumentRepository, IndexTier, PathRecord};
    use crate::db::search_repo::SearchRepository;
    use crate::db::store::{DocumentStore, SqliteDocumentStore};

    #[test]
    fn reclaim_space_shrinks_the_file_after_deleting_indexed_documents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Db::open(&path).unwrap();

        // Enough data that deleting it frees a measurable number of pages -
        // a handful of rows wouldn't move the file size at all (SQLite's
        // page size means small changes get lost in rounding). Populates
        // `content_fts` too (`SearchRepository::index_content`, not just
        // `document_bodies`/`documents`) - FTS5 doesn't reclaim its internal
        // segment storage on plain `DELETE` the way an ordinary table does,
        // only on `optimize` (`reclaim_space`'s doc comment), so skipping it
        // here would miss the dominant real-world contributor to DB size.
        for i in 0..500 {
            let id = format!("doc{i}");
            let body = "본문 발행 채권 절차 규정 ".repeat(2000);
            DocumentRepository::upsert_document(
                &db.conn,
                &DocumentRecord {
                    document_id: id.clone(),
                    file_size: 10_000,
                    text_bytes: 10_000,
                    index_tier: IndexTier::Full,
                },
            )
            .unwrap();
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    path: format!("/watched/{id}.txt"),
                    document_id: id.clone(),
                    filename: format!("{id}.txt"),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
            SqliteDocumentStore { conn: &db.conn }
                .put_body(&id, &body)
                .unwrap();
            SearchRepository::index_content(&db.conn, &id, &body, &body, "").unwrap();
        }
        drop(db);
        let size_before_delete = std::fs::metadata(&path).unwrap().len();

        let db = Db::open(&path).unwrap();
        DocumentRepository::reset_all(&db.conn).unwrap();
        db.reclaim_space().unwrap();
        drop(db);
        let size_after_reclaim = std::fs::metadata(&path).unwrap().len();

        assert!(
            size_after_reclaim < size_before_delete,
            "expected the file to shrink after deleting everything and reclaiming space: \
             before={size_before_delete}, after={size_after_reclaim}"
        );
    }

    /// Reported bug: excluding a watched folder (~5000 files) from indexing
    /// didn't shrink the `.db` file at all. Reproduces with a *partial*
    /// deletion instead of wiping everything (the test above) - this is the
    /// scenario that actually exposed both empirical surprises documented on
    /// `reclaim_space`: `PRAGMA incremental_vacuum(N)` reclaiming roughly
    /// nothing regardless of N, and `content_fts` needing `optimize` before
    /// `VACUUM` can see its dead segment data as reclaimable at all. A
    /// full-DB wipe (`reset_all`, the test above) turned out to mostly hide
    /// both problems, so it alone wasn't enough to catch this.
    #[test]
    fn reclaim_space_shrinks_the_file_after_a_partial_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Db::open(&path).unwrap();

        let body = "본문 발행 채권 절차 규정 이사회 결의 예산 국채 승인 ".repeat(300);
        for i in 0..1000 {
            let id = format!("doc{i}");
            DocumentRepository::upsert_document(
                &db.conn,
                &DocumentRecord {
                    document_id: id.clone(),
                    file_size: 10_000,
                    text_bytes: 10_000,
                    index_tier: IndexTier::Full,
                },
            )
            .unwrap();
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    // Half under a folder that's about to be excluded, half
                    // under one that stays - mirrors "removing one watched
                    // folder out of several".
                    path: format!(
                        "/watched/{}/{id}.txt",
                        if i % 2 == 0 { "excluded" } else { "kept" }
                    ),
                    document_id: id.clone(),
                    filename: format!("{id}.txt"),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
            SqliteDocumentStore { conn: &db.conn }
                .put_body(&id, &body)
                .unwrap();
            SearchRepository::index_content(&db.conn, &id, &body, &body, "").unwrap();
        }
        db.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
        let size_full = std::fs::metadata(&path).unwrap().len();

        DocumentRepository::remove_paths_under(&db.conn, "/watched/excluded").unwrap();
        db.reclaim_space().unwrap();
        let size_after = std::fs::metadata(&path).unwrap().len();

        // Half the documents were removed, so the file should be close to
        // half its original size - a much stricter bound than "just some
        // shrinkage", to actually catch a reclaim mechanism that technically
        // frees a little but not nearly what it should (exactly how the
        // reported bug looked with an unqualified `< size_full` check).
        assert!(
            size_after < size_full * 3 / 4,
            "expected the file to shrink close to half after removing half the \
             documents: full={size_full}, after={size_after}"
        );
    }
}
