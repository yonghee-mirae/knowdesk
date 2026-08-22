//! `SearchService` — search (`docs/08_API_Contracts.md`).

pub mod parser;
pub mod service;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    Filename,
    Content,
}

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    pub mode: SearchMode,
    /// `0` (or any negative value) means no limit - every match is returned
    /// (`SqliteSearchService::search` normalizes it to SQLite's own
    /// "negative `LIMIT` means unlimited" convention before querying).
    pub limit: i64,
}

/// How this hit was matched. Filename mode never uses morphological
/// analysis, so it's always `Exact`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    /// Matched on the literal text (body/bigram, etc.).
    Exact,
    /// Matched only via query expansion (Kiwi morphological analysis) — not
    /// matched by the literal text.
    Morphological,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: String,
    pub filename: String,
    /// Generated on the fly from the DB's stored text at search time
    /// (`docs/01_KnowDesk_PRD.md` F-03). No separate cache.
    pub snippet: Option<String>,
    pub rank: f64,
    pub match_kind: MatchKind,
    pub extension: String,
    /// RFC3339 timestamp string, as stored in `paths.modified_at`.
    pub modified_at: Option<String>,
    /// `FULL` | `META` | `SKIP` (`docs/04_Data_Model.md`) — lets the UI show a
    /// "content not indexed" badge for `META` hits instead of a snippet.
    pub index_tier: String,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
}

#[derive(thiserror::Error, Debug)]
pub enum SearchError {
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
}

pub trait SearchService {
    fn search(&self, request: &SearchRequest) -> Result<SearchResult, SearchError>;
}
