//! Repository for the `documents` / `paths` tables.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::search_repo::SearchRepository;
use crate::db::store::{DocumentStore, SqliteDocumentStore};
use crate::DocId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexTier {
    Full,
    Meta,
    Skip,
}

impl IndexTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexTier::Full => "FULL",
            IndexTier::Meta => "META",
            IndexTier::Skip => "SKIP",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "FULL" => Some(IndexTier::Full),
            "META" => Some(IndexTier::Meta),
            "SKIP" => Some(IndexTier::Skip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentRecord {
    pub document_id: DocId,
    pub file_size: i64,
    pub text_bytes: i64,
    pub index_tier: IndexTier,
}

#[derive(Debug, Clone)]
pub struct PathRecord {
    pub path: String,
    pub document_id: DocId,
    pub filename: String,
    pub extension: String,
    pub modified_at: Option<String>,
}

pub struct DocumentRepository;

impl DocumentRepository {
    pub fn upsert_document(conn: &Connection, doc: &DocumentRecord) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO documents
                (document_id, file_size, text_bytes, index_tier, indexed_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(document_id) DO UPDATE SET
                file_size = excluded.file_size,
                text_bytes = excluded.text_bytes,
                index_tier = excluded.index_tier,
                indexed_at = excluded.indexed_at",
            params![
                doc.document_id,
                doc.file_size,
                doc.text_bytes,
                doc.index_tier.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn exists(conn: &Connection, document_id: &str) -> rusqlite::Result<bool> {
        Self::get_tier(conn, document_id).map(|t| t.is_some())
    }

    /// Looks up the tier of an already-indexed document. If identical content (by hash)
    /// already exists, this value is reused as-is without re-extraction.
    pub fn get_tier(conn: &Connection, document_id: &str) -> rusqlite::Result<Option<IndexTier>> {
        conn.query_row(
            "SELECT index_tier FROM documents WHERE document_id = ?1",
            params![document_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|opt| opt.and_then(|s| IndexTier::parse(&s)))
    }

    /// If `path.path` was already indexed, repointing it at different content (`document_id`
    /// changed - e.g. the file was edited while the app wasn't running to see it as a live
    /// change, then re-scanned on the next start) can orphan the *previous* document_id this
    /// path used to reference. That old document is cleaned up the same way `remove_path`
    /// does for a path that disappears outright - otherwise its `documents`/`content_fts`/
    /// `document_bodies` rows would stay stranded forever (nothing else references them, and
    /// nothing else would ever notice - the same `document_id` never gets re-extracted once
    /// it already has a `documents` row, see `pipeline::index_file`).
    pub fn upsert_path(conn: &Connection, path: &PathRecord) -> rusqlite::Result<()> {
        let previous_document_id: Option<String> = conn
            .query_row(
                "SELECT document_id FROM paths WHERE path = ?1",
                params![path.path],
                |row| row.get(0),
            )
            .optional()?;

        conn.execute(
            "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET
                document_id = excluded.document_id,
                filename = excluded.filename,
                extension = excluded.extension,
                modified_at = excluded.modified_at,
                seen_at = excluded.seen_at",
            params![
                path.path,
                path.document_id,
                path.filename,
                path.extension,
                path.modified_at,
            ],
        )?;

        if let Some(previous_document_id) = previous_document_id {
            if previous_document_id != path.document_id {
                Self::cleanup_if_orphaned(conn, &previous_document_id)?;
            }
        }
        Ok(())
    }

    /// Called when a path disappears (file watching, B4). Deletes that path, and if the
    /// document it referenced no longer points to any path (orphaned), also cleans up
    /// `documents`/`content_fts`/`document_bodies`. If a copy with identical content exists
    /// at another path (e.g. a duplicated file), that document is left in place.
    ///
    /// Distinguishing a network drive going offline all at once from an actual deletion is
    /// still an open issue (`KnowDesk_추가검토사항.md` D-1) — for now, if a single watched
    /// path disappears, it is treated as a deletion outright.
    pub fn remove_path(conn: &Connection, path: &str) -> rusqlite::Result<Option<DocId>> {
        let document_id: Option<String> = conn
            .query_row(
                "SELECT document_id FROM paths WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()?;
        let Some(document_id) = document_id else {
            return Ok(None);
        };

        conn.execute("DELETE FROM paths WHERE path = ?1", params![path])?;
        Self::cleanup_if_orphaned(conn, &document_id)?;
        Ok(Some(document_id))
    }

    /// Shared by `remove_path` and `upsert_path`: if no `paths` row references `document_id`
    /// any more, cascades the cleanup to `content_fts`/`document_bodies`/`documents`. A no-op
    /// if another path still points at the same content (e.g. a duplicated file).
    fn cleanup_if_orphaned(conn: &Connection, document_id: &str) -> rusqlite::Result<()> {
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM paths WHERE document_id = ?1",
            params![document_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            SearchRepository::remove_content(conn, document_id)?;
            conn.execute(
                "DELETE FROM document_bodies WHERE document_id = ?1",
                params![document_id],
            )?;
            conn.execute(
                "DELETE FROM documents WHERE document_id = ?1",
                params![document_id],
            )?;
        }
        Ok(())
    }

    /// Called when `dir_path` disappears and turns out to have been a directory (a whole
    /// watched folder, or a folder nested inside one, deleted at once) rather than a single
    /// file - `remove_path` alone only matches one exact `paths.path` value, so it can't
    /// clean up anything that was indexed underneath a deleted directory. Removes every path
    /// nested under `dir_path` (matched via `Path::starts_with`, not a raw string prefix, so
    /// e.g. `/a/b` doesn't also match a sibling like `/a/bc`), cascading exactly like
    /// `remove_path` for each one. A no-op if nothing in `paths` is nested under `dir_path` -
    /// the common case where the deleted path really was just a single file, already handled
    /// by the caller's own `remove_path` call (`queue::handle_path`).
    pub fn remove_paths_under(conn: &Connection, dir_path: &str) -> rusqlite::Result<Vec<DocId>> {
        let dir_path = std::path::Path::new(dir_path);
        let mut stmt = conn.prepare("SELECT path FROM paths")?;
        let nested: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|p| std::path::Path::new(p).starts_with(dir_path))
            .collect();

        let mut removed = Vec::new();
        for path in nested {
            if let Some(document_id) = Self::remove_path(conn, &path)? {
                removed.push(document_id);
            }
        }
        Ok(removed)
    }

    /// Purges every already-indexed path that isn't nested under any of `watched_dirs`
    /// (already-canonicalized, same representation stored in `paths` - see
    /// `index::canonical_path`). Unlike `remove_paths_under`/`prune_missing_paths_under`,
    /// this doesn't target one specific folder - it reconciles the *whole* DB against the
    /// current `config.watched_folders` in one pass, which is what makes it correct even
    /// when a folder was removed from `settings.json` while the app wasn't running at all:
    /// `src-tauri`'s `apply_folder_diff` only tracks folder additions/removals relative to
    /// its own in-memory `current` list, which starts empty on every fresh process - so a
    /// folder removed during that downtime is neither "added" nor "removed" from that
    /// diff's point of view (it was never in `current` to begin with), and would otherwise
    /// never be noticed. Calling this unconditionally, independent of that diff, covers
    /// both the live-removal case and this startup-while-closed one with a single
    /// mechanism. A no-op if every indexed path is still under some watched folder.
    pub fn prune_paths_outside_watched(
        conn: &Connection,
        watched_dirs: &[std::path::PathBuf],
    ) -> rusqlite::Result<Vec<DocId>> {
        let mut stmt = conn.prepare("SELECT path FROM paths")?;
        let outside: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|p| {
                let p = std::path::Path::new(p);
                !watched_dirs.iter().any(|dir| p.starts_with(dir))
            })
            .collect();

        let mut removed = Vec::new();
        for path in outside {
            if let Some(document_id) = Self::remove_path(conn, &path)? {
                removed.push(document_id);
            }
        }
        Ok(removed)
    }

    /// Reconciles already-indexed paths nested under `dir_path` against the filesystem,
    /// removing (`remove_path`) any that no longer exist. Complements the initial/startup
    /// scan (`walker::scan` + `IndexPipeline::index_directory`) - that scan only ever
    /// adds/updates whatever files it currently finds, so it has no way to notice a
    /// previously-indexed file that's gone now: a deleted file simply isn't in its results
    /// at all, rather than showing up as something to remove. This matters because a
    /// deletion that happens while the app isn't running produces no live `notify` event for
    /// `queue::handle_path`/`remove_paths_under` to catch - the very next startup scan of that
    /// folder is the only chance to notice it (`src-tauri`'s `apply_folder_diff` calls this
    /// for every scanned folder, not just ones with a `notify` event pending).
    pub fn prune_missing_paths_under(conn: &Connection, dir_path: &str) -> rusqlite::Result<Vec<DocId>> {
        let dir_path = std::path::Path::new(dir_path);
        let mut stmt = conn.prepare("SELECT path FROM paths")?;
        let missing: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|p| {
                let p = std::path::Path::new(p);
                p.starts_with(dir_path) && !p.exists()
            })
            .collect();

        let mut removed = Vec::new();
        for path in missing {
            if let Some(document_id) = Self::remove_path(conn, &path)? {
                removed.push(document_id);
            }
        }
        Ok(removed)
    }

    /// Index summary: list of (tier, count). Backs the summary text format in
    /// `추가검토사항.md` B-3.
    pub fn count_by_tier(conn: &Connection) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt =
            conn.prepare("SELECT index_tier, COUNT(*) FROM documents GROUP BY index_tier")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Most recent `indexed_at` timestamp across all documents, or `None` if
    /// nothing has been indexed yet. Backs the "마지막 색인 시각" line in the
    /// statistics summary (TASK-901, `12_UI_Spec.md` C5).
    pub fn last_indexed_at(conn: &Connection) -> rusqlite::Result<Option<String>> {
        conn.query_row("SELECT MAX(indexed_at) FROM documents", [], |row| {
            row.get(0)
        })
    }

    /// Wipes every indexed document (`paths`/`document_bodies`/`content_fts`/
    /// `documents`) while leaving the schema itself intact - "색인 초기화"
    /// (Reset Index, tray menu action). The caller is responsible for
    /// re-scanning the watched folders afterward; this only clears the DB.
    pub fn reset_all(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM paths", [])?;
        conn.execute("DELETE FROM document_bodies", [])?;
        conn.execute("DELETE FROM content_fts", [])?;
        conn.execute("DELETE FROM documents", [])?;
        Ok(())
    }

    /// First `max_chars` characters of the body text stored for whatever
    /// document `path` currently resolves to - a preview for a hit with no
    /// snippet (a filter-only query, or filename mode, neither of which has a
    /// keyword to build a snippet around, `docs/12_UI_Spec.md` C2). `None` if
    /// `path` isn't indexed, or its document has no stored body at all (a
    /// META-tier document, whose content was never extracted in the first
    /// place - `core::index::pipeline`).
    pub fn body_preview(
        conn: &Connection,
        path: &str,
        max_chars: usize,
    ) -> rusqlite::Result<Option<String>> {
        let document_id: Option<String> = conn
            .query_row(
                "SELECT document_id FROM paths WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .optional()?;
        let Some(document_id) = document_id else {
            return Ok(None);
        };
        let body = SqliteDocumentStore { conn }.get_body(&document_id)?;
        Ok(body.map(|b| truncate_chars(&b, max_chars)))
    }
}

/// Truncates `text` to at most `max_chars` characters (not bytes - Korean
/// text is multi-byte UTF-8), appending `...` if anything was actually cut.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn upsert_path_cleans_up_the_previous_document_when_repointed_to_new_content() {
        // Simulates a file edited while the app wasn't running, then
        // re-scanned on the next start (`pipeline::index_file` computes a
        // new content-hash `document_id` and calls `upsert_path` to repoint
        // the same path string at it - see that function's doc comment).
        let db = Db::open_in_memory().unwrap();
        for id in ["old_content", "new_content"] {
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
            SqliteDocumentStore { conn: &db.conn }
                .put_body(id, "body")
                .unwrap();
            SearchRepository::index_content(&db.conn, id, "body", "body", "").unwrap();
        }

        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/report.txt".to_string(),
                document_id: "old_content".to_string(),
                filename: "report.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        // Re-scanned: same path, but the file's content (and so its
        // document_id) changed underneath it.
        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/report.txt".to_string(),
                document_id: "new_content".to_string(),
                filename: "report.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        assert!(
            !DocumentRepository::exists(&db.conn, "old_content").unwrap(),
            "the previous document_id must be cleaned up once no path references it any more"
        );
        assert!(DocumentRepository::exists(&db.conn, "new_content").unwrap());
        let orphan_body: Option<String> = db
            .conn
            .query_row(
                "SELECT body FROM document_bodies WHERE document_id = 'old_content'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(orphan_body, None, "document_bodies must also be cleaned up");
    }

    #[test]
    fn upsert_path_leaves_a_document_alone_while_another_path_still_references_it() {
        // A duplicated file (same content at two paths) - repointing one of
        // them elsewhere must not delete the shared document while the
        // other path still references it.
        let db = Db::open_in_memory().unwrap();
        for id in ["shared", "new_content"] {
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
        for path in ["/a/one.txt", "/a/copy.txt"] {
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    path: path.to_string(),
                    document_id: "shared".to_string(),
                    filename: path.rsplit('/').next().unwrap().to_string(),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
        }

        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/one.txt".to_string(),
                document_id: "new_content".to_string(),
                filename: "one.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        assert!(
            DocumentRepository::exists(&db.conn, "shared").unwrap(),
            "/a/copy.txt still references it - must not be deleted"
        );
    }

    #[test]
    fn reset_all_clears_documents_and_paths() {
        let db = Db::open_in_memory().unwrap();
        DocumentRepository::upsert_document(
            &db.conn,
            &DocumentRecord {
                document_id: "abc".to_string(),
                file_size: 10,
                text_bytes: 5,
                index_tier: IndexTier::Full,
            },
        )
        .unwrap();
        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/b.txt".to_string(),
                document_id: "abc".to_string(),
                filename: "b.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        DocumentRepository::reset_all(&db.conn).unwrap();

        assert!(DocumentRepository::count_by_tier(&db.conn)
            .unwrap()
            .is_empty());
        assert!(!DocumentRepository::exists(&db.conn, "abc").unwrap());
    }

    #[test]
    fn remove_paths_under_purges_nested_paths_but_not_a_same_prefixed_sibling() {
        let db = Db::open_in_memory().unwrap();
        for (id, path) in [
            ("doc_a", "/root/folder/a.txt"),
            ("doc_b", "/root/folder/sub/b.txt"),
            // Same string prefix as "/root/folder" but a different sibling
            // directory - must survive (`Path::starts_with` compares whole
            // components, not raw string prefixes).
            ("doc_c", "/root/folder2/c.txt"),
        ] {
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
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    path: path.to_string(),
                    document_id: id.to_string(),
                    filename: path.rsplit('/').next().unwrap().to_string(),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
        }

        let removed = DocumentRepository::remove_paths_under(&db.conn, "/root/folder").unwrap();
        assert_eq!(removed.len(), 2, "removed: {:?}", removed);
        assert!(!DocumentRepository::exists(&db.conn, "doc_a").unwrap());
        assert!(!DocumentRepository::exists(&db.conn, "doc_b").unwrap());
        assert!(
            DocumentRepository::exists(&db.conn, "doc_c").unwrap(),
            "a sibling folder sharing a string prefix must not be affected"
        );
    }

    #[test]
    fn prune_paths_outside_watched_removes_everything_not_under_the_given_dirs() {
        let db = Db::open_in_memory().unwrap();
        for (id, path) in [
            ("doc_kept", "/watched/a.txt"),
            ("doc_kept_nested", "/watched/sub/b.txt"),
            ("doc_dropped", "/no_longer_watched/c.txt"),
        ] {
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
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    path: path.to_string(),
                    document_id: id.to_string(),
                    filename: path.rsplit('/').next().unwrap().to_string(),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
        }

        let removed = DocumentRepository::prune_paths_outside_watched(
            &db.conn,
            &[std::path::PathBuf::from("/watched")],
        )
        .unwrap();
        assert_eq!(removed, vec!["doc_dropped".to_string()]);
        assert!(DocumentRepository::exists(&db.conn, "doc_kept").unwrap());
        assert!(DocumentRepository::exists(&db.conn, "doc_kept_nested").unwrap());
        assert!(!DocumentRepository::exists(&db.conn, "doc_dropped").unwrap());
    }

    #[test]
    fn prune_paths_outside_watched_with_no_watched_dirs_removes_everything() {
        // `watched_folders` cleared entirely - nothing is watched any more,
        // so every indexed path is "outside" and gets purged.
        let db = Db::open_in_memory().unwrap();
        DocumentRepository::upsert_document(
            &db.conn,
            &DocumentRecord {
                document_id: "doc".to_string(),
                file_size: 10,
                text_bytes: 5,
                index_tier: IndexTier::Full,
            },
        )
        .unwrap();
        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/b.txt".to_string(),
                document_id: "doc".to_string(),
                filename: "b.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        let removed = DocumentRepository::prune_paths_outside_watched(&db.conn, &[]).unwrap();
        assert_eq!(removed, vec!["doc".to_string()]);
    }

    #[test]
    fn prune_missing_paths_under_removes_only_paths_actually_gone_from_disk() {
        // Real files on disk (not just DB rows) - `prune_missing_paths_under`
        // checks `Path::exists()`, unlike `remove_paths_under` above which
        // removes everything nested under a directory unconditionally.
        let dir = tempfile::tempdir().unwrap();
        let still_here = dir.path().join("still_here.txt");
        let gone = dir.path().join("gone.txt");
        std::fs::write(&still_here, "content").unwrap();
        std::fs::write(&gone, "content").unwrap();

        let db = Db::open_in_memory().unwrap();
        for (id, path) in [("doc_here", &still_here), ("doc_gone", &gone)] {
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
            DocumentRepository::upsert_path(
                &db.conn,
                &PathRecord {
                    path: path.to_string_lossy().to_string(),
                    document_id: id.to_string(),
                    filename: path.file_name().unwrap().to_string_lossy().to_string(),
                    extension: "txt".to_string(),
                    modified_at: None,
                },
            )
            .unwrap();
        }

        // Only deleted from disk after being indexed - same "app wasn't
        // running to see it live" scenario `apply_folder_diff`'s startup
        // scan is meant to catch.
        std::fs::remove_file(&gone).unwrap();

        let removed =
            DocumentRepository::prune_missing_paths_under(&db.conn, &dir.path().to_string_lossy())
                .unwrap();
        assert_eq!(removed, vec!["doc_gone".to_string()]);
        assert!(
            DocumentRepository::exists(&db.conn, "doc_here").unwrap(),
            "a path still present on disk must not be pruned"
        );
        assert!(!DocumentRepository::exists(&db.conn, "doc_gone").unwrap());
    }

    #[test]
    fn last_indexed_at_is_none_when_empty_and_the_latest_timestamp_otherwise() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(DocumentRepository::last_indexed_at(&db.conn).unwrap(), None);

        DocumentRepository::upsert_document(
            &db.conn,
            &DocumentRecord {
                document_id: "abc".to_string(),
                file_size: 10,
                text_bytes: 5,
                index_tier: IndexTier::Full,
            },
        )
        .unwrap();

        assert!(DocumentRepository::last_indexed_at(&db.conn)
            .unwrap()
            .is_some());
    }

    #[test]
    fn body_preview_truncates_by_character_and_handles_missing_cases() {
        let db = Db::open_in_memory().unwrap();

        // Unknown path - not indexed at all.
        assert_eq!(
            DocumentRepository::body_preview(&db.conn, "/a/b.txt", 10).unwrap(),
            None
        );

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
        DocumentRepository::upsert_path(
            &db.conn,
            &PathRecord {
                path: "/a/규정.txt".to_string(),
                document_id: "full1".to_string(),
                filename: "규정.txt".to_string(),
                extension: "txt".to_string(),
                modified_at: None,
            },
        )
        .unwrap();

        // Indexed, but META (no body ever stored) - `put_body` deliberately
        // not called here, matching `extract_and_index`'s `Err` branch.
        assert_eq!(
            DocumentRepository::body_preview(&db.conn, "/a/규정.txt", 10).unwrap(),
            None
        );

        SqliteDocumentStore { conn: &db.conn }
            .put_body("full1", "채권 발행 절차를 규정한다")
            .unwrap();

        // Longer than max_chars - truncated (by character count) with a
        // trailing "...".
        assert_eq!(
            DocumentRepository::body_preview(&db.conn, "/a/규정.txt", 4).unwrap(),
            Some("채권 발...".to_string())
        );
        // Shorter than max_chars - returned whole, no "...".
        assert_eq!(
            DocumentRepository::body_preview(&db.conn, "/a/규정.txt", 100).unwrap(),
            Some("채권 발행 절차를 규정한다".to_string())
        );
    }
}
