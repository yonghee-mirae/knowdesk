use rusqlite::Connection;

use super::parser::parse;
use super::{
    SearchError, SearchHit, SearchMode, SearchRequest, SearchResult,
    SearchService as SearchServiceTrait,
};
use crate::db::search_repo::SearchRepository;

pub struct SqliteSearchService<'a> {
    pub conn: &'a Connection,
}

impl<'a> SearchServiceTrait for SqliteSearchService<'a> {
    fn search(&self, request: &SearchRequest) -> Result<SearchResult, SearchError> {
        let parsed = parse(&request.query);

        let rows = match request.mode {
            SearchMode::Filename => SearchRepository::search_filename(
                self.conn,
                &parsed.match_expr,
                &parsed.filters,
                request.limit,
            )?,
            SearchMode::Content => SearchRepository::search_content(
                self.conn,
                &parsed.match_expr,
                &parsed.filters,
                request.limit,
            )?,
        };

        let hits = rows
            .into_iter()
            .map(|row| SearchHit {
                path: row.path,
                filename: row.filename,
                snippet: row.snippet,
                rank: row.rank,
            })
            .collect();

        Ok(SearchResult { hits })
    }
}
