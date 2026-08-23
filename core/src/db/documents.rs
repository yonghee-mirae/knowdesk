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

    pub fn upsert_path(conn: &Connection, path: &PathRecord) -> rusqlite::Result<()> {
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

        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM paths WHERE document_id = ?1",
            params![document_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            SearchRepository::remove_content(conn, &document_id)?;
            conn.execute(
                "DELETE FROM document_bodies WHERE document_id = ?1",
                params![document_id],
            )?;
            conn.execute(
                "DELETE FROM documents WHERE document_id = ?1",
                params![document_id],
            )?;
        }
        Ok(Some(document_id))
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
