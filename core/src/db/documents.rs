//! Repository for the `documents` / `paths` tables.

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::search_repo::SearchRepository;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexStatus {
    Indexed,
    MetaIndexed,
    Failed,
}

impl IndexStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexStatus::Indexed => "INDEXED",
            IndexStatus::MetaIndexed => "META_INDEXED",
            IndexStatus::Failed => "FAILED",
        }
    }
}

/// `demotion_reason` values from `docs/04_Data_Model.md`. `EMPTY_TEXT` is omitted since
/// it's still an open item not yet reflected in the docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemotionReason {
    Drm,
    Corrupt,
    Encrypted,
    ParseFail,
}

impl DemotionReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            DemotionReason::Drm => "DRM",
            DemotionReason::Corrupt => "CORRUPT",
            DemotionReason::Encrypted => "ENCRYPTED",
            DemotionReason::ParseFail => "PARSE_FAIL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DocumentRecord {
    pub document_id: DocId,
    pub file_size: i64,
    pub text_bytes: i64,
    pub index_tier: IndexTier,
    pub index_status: IndexStatus,
    pub demotion_reason: Option<DemotionReason>,
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
                (document_id, file_size, text_bytes, index_tier, index_status, demotion_reason, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(document_id) DO UPDATE SET
                file_size = excluded.file_size,
                text_bytes = excluded.text_bytes,
                index_tier = excluded.index_tier,
                index_status = excluded.index_status,
                demotion_reason = excluded.demotion_reason,
                indexed_at = excluded.indexed_at",
            params![
                doc.document_id,
                doc.file_size,
                doc.text_bytes,
                doc.index_tier.as_str(),
                doc.index_status.as_str(),
                doc.demotion_reason.map(|r| r.as_str()),
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

    pub fn count_by_demotion_reason(conn: &Connection) -> rusqlite::Result<Vec<(String, i64)>> {
        let mut stmt = conn.prepare(
            "SELECT demotion_reason, COUNT(*) FROM documents
             WHERE demotion_reason IS NOT NULL GROUP BY demotion_reason",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
