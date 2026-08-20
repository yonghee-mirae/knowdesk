//! `SearchService` — 검색 (`docs/08_API_Contracts.md`).

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
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: String,
    pub filename: String,
    /// 검색 시 DB 원문에서 즉시 생성한다 (`docs/01_KnowDesk_PRD.md` F-03). 별도 캐시 없음.
    pub snippet: Option<String>,
    pub rank: f64,
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
