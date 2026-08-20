//! SQLite 스키마. `docs/04_Data_Model.md`를 단일 출처로 한다.
//!
//! `filename_fts`/`content_fts`에는 문서 모델 문서에 없는 `UNINDEXED` 연결 컬럼
//! (`path`, `document_id`)을 추가했다 — FTS5 결과를 `documents`/`paths`로 되짚어
//! 조인하기 위한 구현상 필수 요소이며, 검색 가능한 컬럼(`filename`/`body`/`morph`)
//! 자체는 문서 정의 그대로다.

pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS documents
(
    document_id TEXT PRIMARY KEY,   -- SHA256(content)

    file_size INTEGER,
    text_bytes INTEGER,             -- 추출된 본문 크기 (DB 용량 추정/통계용)

    index_tier TEXT NOT NULL,       -- FULL | META | SKIP
    index_status TEXT NOT NULL,     -- 상태 머신, `docs/04_Data_Model.md` 참조

    demotion_reason TEXT,           -- DRM | CORRUPT | ENCRYPTED | PARSE_FAIL

    drm_status TEXT,                -- NON_DRM | DRM | UNKNOWN
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,

    content_stored INTEGER NOT NULL DEFAULT 1,  -- 1=원문 저장, 0=압축/미저장

    indexed_at TEXT
);

CREATE TABLE IF NOT EXISTS paths
(
    path TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(document_id),

    filename TEXT NOT NULL,
    extension TEXT NOT NULL,

    modified_at TEXT,
    seen_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_paths_document_id ON paths(document_id);

CREATE TABLE IF NOT EXISTS document_bodies
(
    document_id TEXT PRIMARY KEY REFERENCES documents(document_id),
    body TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS filename_fts USING fts5(
    filename,
    path UNINDEXED
);

CREATE VIRTUAL TABLE IF NOT EXISTS content_fts USING fts5(
    body,
    morph,
    document_id UNINDEXED
);
"#;
