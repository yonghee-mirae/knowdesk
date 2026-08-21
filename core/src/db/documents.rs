//! `documents` / `paths` 테이블 저장소.

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

/// `docs/04_Data_Model.md`의 `demotion_reason` 값. `EMPTY_TEXT`는 아직 문서에
/// 반영되지 않은 미결 항목이라 포함하지 않는다.
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

    /// 이미 색인된 문서의 계층을 조회한다. 동일 내용(hash)이 이미 있으면
    /// 재추출 없이 이 값을 그대로 재사용한다.
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

    /// 경로가 사라졌을 때 호출한다 (파일 감시, B4). 그 경로를 지우고, 참조하던
    /// 문서를 더 이상 아무 경로도 가리키지 않게 되면(orphan) `documents`/
    /// `content_fts`/`document_bodies`까지 함께 정리한다. 동일 내용의 사본이
    /// 다른 경로에도 있으면(예: 파일 복사본) 그 문서는 그대로 남는다.
    ///
    /// 네트워크 드라이브가 한꺼번에 오프라인되는 것과 실제 삭제를 구분하는 문제는
    /// 미결 상태다(`KnowDesk_추가검토사항.md` D-1) — 지금은 감시 대상 경로 하나가
    /// 사라지면 그대로 삭제로 처리한다.
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
        SearchRepository::remove_filename(conn, path)?;

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

    /// 색인 요약: (tier, 건수) 목록. `추가검토사항.md` B-3의 요약 문구 형식을 위한 것.
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
