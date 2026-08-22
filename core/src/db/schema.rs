//! SQLite schema. `docs/04_Data_Model.md` is the single source of truth.
//!
//! `filename_fts`/`content_fts` add `UNINDEXED` linking columns (`path`, `document_id`)
//! that aren't in the data model doc — these are an implementation necessity for joining
//! FTS5 results back to `documents`/`paths`; the searchable columns themselves
//! (`filename`/`body`/`morph`) match the document definition as-is.
//!
//! `content_fts.morph_kiwi` is a column that wasn't in the v1.1 schema (added in the Phase B2
//! redesign). The roles are split: `morph` (bigram) is the default tokenizer, always populated;
//! `morph_kiwi` is the secondary tokenizer, populated only when Kiwi is available —
//! see `04_Data_Model.md` for details.

pub const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS documents
(
    document_id TEXT PRIMARY KEY,   -- SHA256(content)

    file_size INTEGER,
    text_bytes INTEGER,             -- extracted body size (for DB size estimation/stats)

    index_tier TEXT NOT NULL,       -- FULL | META | SKIP
    index_status TEXT NOT NULL,     -- state machine, see `docs/04_Data_Model.md`

    demotion_reason TEXT,           -- DRM | CORRUPT | ENCRYPTED | PARSE_FAIL

    drm_status TEXT,                -- NON_DRM | DRM | UNKNOWN
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TEXT,

    content_stored INTEGER NOT NULL DEFAULT 1,  -- 1=original text stored, 0=compressed/not stored

    indexed_at TEXT
);
-- `index_status`/`demotion_reason`/`drm_status`/`retry_count`/`last_attempt_at`/
-- `content_stored` are dropped by `db::migrate` MIGRATIONS v3 (2026-08-24 decision):
-- none of them was ever actually read or written by any code path (`index_status`
-- duplicated `index_tier` one-to-one and was write-only; `demotion_reason` only ever
-- held `PARSE_FAIL` or nothing, and distinguishing DRM/CORRUPT/ENCRYPTED was decided
-- unnecessary; `drm_status`/`retry_count`/`last_attempt_at`/`content_stored` backed
-- features - a state machine, a retry policy, compressed storage - that were never
-- built). Left as-is here rather than edited out, since this is the historical
-- record of what v1 actually applied.

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
-- Dropped by `db::migrate` MIGRATIONS v2 (`docs/05_Search_Language_v1.md`, Filename
-- Mode) - filename search moved to a plain SQL substring match on `paths.filename`
-- instead. Left as-is here rather than edited out, since this is the historical
-- record of what v1 actually applied.

CREATE VIRTUAL TABLE IF NOT EXISTS content_fts USING fts5(
    body,
    morph,
    morph_kiwi,
    document_id UNINDEXED
);
"#;
