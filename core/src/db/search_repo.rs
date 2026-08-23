//! Indexing and querying for `content_fts`, plus the plain-substring filename search
//! (`search_filename`) that queries `paths`/`documents` directly instead.
//!
//! bm25 column weights: `body` dominates, with `morph` (bigram, the default tokenizer,
//! always populated) and `morph_kiwi` (Kiwi, the secondary tokenizer, populated only when
//! available) added as secondary signals. Kiwi is more precise than bigram, so it gets a
//! higher weight — all three values are provisional pending real measurement and subject to
//! tuning. Filters (`ext`/`path`/`tier`/`drm`/`modified`) are applied at the SQL level by
//! joining against `documents`/`paths`.

use rusqlite::types::ToSql;
use rusqlite::{params, Connection};

use crate::search::parser::Filters;

pub const CONTENT_BODY_WEIGHT: f64 = 1.0;
pub const CONTENT_MORPH_WEIGHT: f64 = 0.3;
pub const CONTENT_MORPH_KIWI_WEIGHT: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct SearchRow {
    pub path: String,
    pub filename: String,
    pub snippet: Option<String>,
    pub rank: f64,
    /// Only populated in content mode — needed later to determine whether the hit came
    /// from query expansion (`morph_kiwi`) matching (`search::service`).
    pub document_id: Option<String>,
    pub extension: String,
    pub modified_at: Option<String>,
    pub index_tier: String,
}

pub struct SearchRepository;

impl SearchRepository {
    /// Removes the content index entry for a single document (called by
    /// `DocumentRepository::remove_path` for cleanup, when no path referencing this
    /// document remains).
    pub fn remove_content(conn: &Connection, document_id: &str) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM content_fts WHERE document_id = ?1",
            params![document_id],
        )?;
        Ok(())
    }

    /// Index content. Since documents are keyed by content (document_id), delete and
    /// re-insert the existing row. `morph_kiwi` is an empty string in environments where
    /// Kiwi is unavailable.
    pub fn index_content(
        conn: &Connection,
        document_id: &str,
        body: &str,
        morph: &str,
        morph_kiwi: &str,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM content_fts WHERE document_id = ?1",
            params![document_id],
        )?;
        conn.execute(
            "INSERT INTO content_fts (body, morph, morph_kiwi, document_id) VALUES (?1, ?2, ?3, ?4)",
            params![body, morph, morph_kiwi, document_id],
        )?;
        Ok(())
    }

    /// Number of `content_fts` rows with a non-empty `morph_kiwi` - i.e.
    /// FULL-tier documents actually morphologically analyzed by Kiwi, out of
    /// all full-text-indexed documents (every `content_fts` row is FULL tier
    /// by construction - `index_content` above is only ever called from the
    /// `Ok` branch of `pipeline::extract_and_index`). Used by "Statistics"
    /// (tray menu, `src-tauri`'s `compute_stats`) to show how much of the
    /// full-text index actually got Kiwi's analysis vs. bigram-only (Kiwi
    /// unavailable, or `enable_morphological_analysis` off - `core::config`).
    pub fn count_kiwi_analyzed(conn: &Connection) -> rusqlite::Result<i64> {
        conn.query_row(
            "SELECT COUNT(*) FROM content_fts WHERE morph_kiwi != ''",
            [],
            |row| row.get(0),
        )
    }

    /// `needles` are plain literal substrings (`search::parser::parse_filename`) -
    /// a filename matches only if it contains every one of them, case-insensitively
    /// (ANDed). Filenames aren't indexed with FTS5 at all (the old `filename_fts`
    /// table was dropped, `db::migrate` MIGRATIONS v2): FTS5 only supports
    /// trailing-prefix wildcard search (`발행*`) and otherwise matches whole
    /// tokens, which for CJK filenames with no natural word boundaries often
    /// means "the whole filename" rather than any substring within it. A plain
    /// SQL `LIKE '%...%'` has no such limitation.
    pub fn search_filename(
        conn: &Connection,
        needles: &[String],
        filters: &Filters,
        limit: i64,
    ) -> rusqlite::Result<Vec<SearchRow>> {
        // No keyword (e.g. a bare `x:pdf`) - there's no relevance to rank
        // anyway, so just browse `paths`/`documents` directly.
        if needles.is_empty() {
            return search_filename_filters_only(conn, filters, limit);
        }

        let mut sql = String::from(
            "SELECT p.path, p.filename, NULL, 0.0,
                    p.extension, p.modified_at, d.index_tier
             FROM paths p
             JOIN documents d ON d.document_id = p.document_id
             WHERE 1 = 1",
        );
        let mut sql_params: Vec<Box<dyn ToSql>> = Vec::new();
        for needle in needles {
            // ESCAPE '\' so a needle containing a literal `%`/`_` (SQL LIKE's
            // own wildcard characters) is matched literally rather than as a
            // wildcard.
            sql.push_str(" AND p.filename LIKE ('%' || ? || '%') ESCAPE '\\'");
            sql_params.push(Box::new(escape_like(needle)));
        }
        push_filter_clauses(&mut sql, &mut sql_params, filters);
        // No FTS5 bm25 rank to sort by anymore - shorter filenames are a
        // reasonable proxy for "tighter" matches, with recency as a tiebreak.
        sql.push_str(" ORDER BY LENGTH(p.filename) ASC, p.modified_at DESC LIMIT ?");
        sql_params.push(Box::new(limit));

        run_search_query(conn, &sql, &sql_params)
    }

    pub fn search_content(
        conn: &Connection,
        match_expr: &str,
        filters: &Filters,
        limit: i64,
    ) -> rusqlite::Result<Vec<SearchRow>> {
        // Same empty-MATCH problem as `search_filename` above.
        if match_expr.trim().is_empty() {
            return search_content_filters_only(conn, filters, limit);
        }

        // bm25()/snippet() can only be called within an fts5 MATCH cursor context.
        // Using GROUP BY would make SQLite go through a sort/aggregate step that loses
        // that context, causing an "unable to use function in the requested context"
        // error — so deduplicating multiple paths per document is handled on the Rust
        // side instead of via GROUP BY.
        let mut sql = String::from(
            "SELECT p.path, p.filename,
                    snippet(content_fts, 0, '>>', '<<', '...', 12),
                    bm25(content_fts, ?, ?, ?),
                    content_fts.document_id,
                    p.extension, p.modified_at, d.index_tier
             FROM content_fts
             JOIN documents d ON d.document_id = content_fts.document_id
             JOIN paths p ON p.document_id = d.document_id
             WHERE content_fts MATCH ?",
        );
        let mut sql_params: Vec<Box<dyn ToSql>> = vec![
            Box::new(CONTENT_BODY_WEIGHT),
            Box::new(CONTENT_MORPH_WEIGHT),
            Box::new(CONTENT_MORPH_KIWI_WEIGHT),
            Box::new(match_expr.to_string()),
        ];
        push_filter_clauses(&mut sql, &mut sql_params, filters);
        sql.push_str(" ORDER BY 4 LIMIT ?");
        // Fetch generously more, by path count, since this is before deduplication.
        sql_params.push(Box::new(limit.saturating_mul(5).max(limit)));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn ToSql> = sql_params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                let document_id: String = row.get(4)?;
                Ok(SearchRow {
                    path: row.get(0)?,
                    filename: row.get(1)?,
                    snippet: row.get(2)?,
                    rank: row.get(3)?,
                    document_id: Some(document_id),
                    extension: row.get(5)?,
                    modified_at: row.get(6)?,
                    index_tier: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        // Enhancing hits whose body (column 0) snippet has no highlight (i.e. matched only
        // via morph/morph_kiwi, e.g. "레이아웃" within "레이아웃과") by finding and
        // highlighting the term directly in the original text is done by `search::service`,
        // since it needs the stored original text (`document_bodies`).
        Ok(dedupe_by_document_id(rows, limit))
    }
}

/// `search_filename` with no keyword (filters only, e.g. bare `x:pdf`). No keyword
/// means no relevance to rank, so this just browses `paths`/`documents` directly,
/// newest-modified first.
fn search_filename_filters_only(
    conn: &Connection,
    filters: &Filters,
    limit: i64,
) -> rusqlite::Result<Vec<SearchRow>> {
    let mut sql = String::from(
        "SELECT p.path, p.filename, NULL, 0.0,
                p.extension, p.modified_at, d.index_tier
         FROM paths p
         JOIN documents d ON d.document_id = p.document_id
         WHERE 1 = 1",
    );
    let mut sql_params: Vec<Box<dyn ToSql>> = Vec::new();
    push_filter_clauses(&mut sql, &mut sql_params, filters);
    sql.push_str(" ORDER BY p.modified_at DESC LIMIT ?");
    sql_params.push(Box::new(limit));

    run_search_query(conn, &sql, &sql_params)
}

/// `search_content` with no keyword — same reasoning as `search_filename_filters_only`,
/// but still dedupes by document since a document can have more than one path.
fn search_content_filters_only(
    conn: &Connection,
    filters: &Filters,
    limit: i64,
) -> rusqlite::Result<Vec<SearchRow>> {
    let mut sql = String::from(
        "SELECT p.path, p.filename, NULL, 0.0, d.document_id,
                p.extension, p.modified_at, d.index_tier
         FROM documents d
         JOIN paths p ON p.document_id = d.document_id
         WHERE 1 = 1",
    );
    let mut sql_params: Vec<Box<dyn ToSql>> = Vec::new();
    push_filter_clauses(&mut sql, &mut sql_params, filters);
    sql.push_str(" ORDER BY p.modified_at DESC LIMIT ?");
    // Fetch generously more, by path count, since this is before deduplication.
    sql_params.push(Box::new(limit.saturating_mul(5).max(limit)));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn ToSql> = sql_params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            let document_id: String = row.get(4)?;
            Ok(SearchRow {
                path: row.get(0)?,
                filename: row.get(1)?,
                snippet: row.get(2)?,
                rank: row.get(3)?,
                document_id: Some(document_id),
                extension: row.get(5)?,
                modified_at: row.get(6)?,
                index_tier: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(dedupe_by_document_id(rows, limit))
}

/// Keeps only the first row per `document_id` (a document can have more than one
/// `paths` row), then truncates to `limit`.
fn dedupe_by_document_id(rows: Vec<SearchRow>, limit: i64) -> Vec<SearchRow> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| seen.insert(row.document_id.clone()))
        .take(limit as usize)
        .collect()
}

/// Escapes SQL LIKE's own wildcard characters (`%`, `_`) and the escape
/// character itself in a needle, so a filename containing one of them
/// literally is matched as literal text, not as a LIKE wildcard. Paired with
/// `ESCAPE '\'` in the query.
fn escape_like(needle: &str) -> String {
    needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn push_filter_clauses(sql: &mut String, params: &mut Vec<Box<dyn ToSql>>, filters: &Filters) {
    if let Some(ext) = &filters.extension {
        sql.push_str(" AND p.extension = ?");
        params.push(Box::new(ext.clone()));
    }
    if let Some(path_contains) = &filters.path_contains {
        sql.push_str(" AND p.path LIKE '%' || ? || '%'");
        params.push(Box::new(path_contains.clone()));
    }
    if let Some(after) = &filters.modified_after {
        sql.push_str(" AND p.modified_at > ?");
        params.push(Box::new(after.clone()));
    }
    if let Some(before) = &filters.modified_before {
        sql.push_str(" AND p.modified_at < ?");
        params.push(Box::new(before.clone()));
    }
    if let Some(on) = &filters.modified_on {
        // `p.modified_at` is a full RFC3339 timestamp (`pipeline::format_system_time`),
        // so a plain `=` against a bare date would never match. `date(...)` extracts
        // just the calendar day from both sides — SQLite's date functions parse
        // RFC3339 natively, so this works regardless of the stored time-of-day.
        sql.push_str(" AND date(p.modified_at) = date(?)");
        params.push(Box::new(on.clone()));
    }
}

fn run_search_query(
    conn: &Connection,
    sql: &str,
    params: &[Box<dyn ToSql>],
) -> rusqlite::Result<Vec<SearchRow>> {
    let mut stmt = conn.prepare(sql)?;
    let param_refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(SearchRow {
                path: row.get(0)?,
                filename: row.get(1)?,
                snippet: row.get(2)?,
                rank: row.get(3)?,
                document_id: None,
                extension: row.get(4)?,
                modified_at: row.get(5)?,
                index_tier: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn count_kiwi_analyzed_counts_only_non_empty_morph_kiwi() {
        let db = Db::open_in_memory().unwrap();
        for id in ["kiwi1", "kiwi2", "bigram_only"] {
            db.conn
                .execute(
                    "INSERT INTO documents (document_id, file_size, text_bytes, index_tier)
                     VALUES (?1, 0, 0, 'FULL')",
                    params![id],
                )
                .unwrap();
        }
        SearchRepository::index_content(&db.conn, "kiwi1", "본문", "본문", "본문").unwrap();
        SearchRepository::index_content(&db.conn, "kiwi2", "본문", "본문", "본문").unwrap();
        // Kiwi unavailable/off (`load_kiwi`/`enable_morphological_analysis`) -
        // `morph_kiwi` left empty, same as `extract_and_index`'s `unwrap_or_default()`.
        SearchRepository::index_content(&db.conn, "bigram_only", "본문", "본문", "").unwrap();

        assert_eq!(SearchRepository::count_kiwi_analyzed(&db.conn).unwrap(), 2);
    }

    #[test]
    fn modified_on_filters_by_calendar_day_regardless_of_time_of_day() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO documents (document_id, file_size, text_bytes, index_tier)
                 VALUES ('doc1', 0, 0, 'FULL')",
                [],
            )
            .unwrap();
        // Deliberately not midnight, to prove the filter matches by calendar day rather
        // than requiring an exact timestamp string match (which would never happen in
        // practice — see the comment on `modified_on` in `search/parser.rs`).
        db.conn
            .execute(
                "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
                 VALUES ('/a/규정.txt', 'doc1', '규정.txt', 'txt', '2026-08-10T14:23:05Z', '2026-08-10T14:23:05Z')",
                [],
            )
            .unwrap();

        let same_day = Filters {
            modified_on: Some("2026-08-10".to_string()),
            ..Filters::default()
        };
        let needles = ["규정".to_string()];
        let hits = SearchRepository::search_filename(&db.conn, &needles, &same_day, 10).unwrap();
        assert_eq!(hits.len(), 1, "hits: {:?}", hits);

        let other_day = Filters {
            modified_on: Some("2026-08-11".to_string()),
            ..Filters::default()
        };
        let hits = SearchRepository::search_filename(&db.conn, &needles, &other_day, 10).unwrap();
        assert!(hits.is_empty(), "hits: {:?}", hits);
    }

    #[test]
    fn search_filename_with_only_filters_and_no_keyword_does_not_crash() {
        // Real bug, confirmed against the actual FTS5 index: a filter-only query (e.g.
        // bare `x:pdf`) strips every token, leaving an empty match_expr. FTS5 rejects
        // `MATCH ''` outright ("fts5: syntax error near ''") — there was no way to
        // browse "everything of this extension" without a keyword.
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO documents (document_id, file_size, text_bytes, index_tier)
             VALUES ('full1', 0, 0, 'FULL')",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO documents (document_id, file_size, text_bytes, index_tier)
             VALUES ('meta1', 0, 0, 'META')",
                [],
            )
            .unwrap();
        db.conn.execute(
            "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
             VALUES ('/a/규정.txt', 'full1', '규정.txt', 'txt', '2026-08-10T00:00:00Z', '2026-08-10T00:00:00Z')",
            [],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
             VALUES ('/a/보안.pdf', 'meta1', '보안.pdf', 'pdf', '2026-08-11T00:00:00Z', '2026-08-11T00:00:00Z')",
            [],
        ).unwrap();

        let pdf_only = Filters {
            extension: Some("pdf".to_string()),
            ..Filters::default()
        };
        let hits = SearchRepository::search_filename(&db.conn, &[], &pdf_only, 10).unwrap();
        assert_eq!(hits.len(), 1, "hits: {:?}", hits);
        assert_eq!(hits[0].filename, "보안.pdf");

        // No filters at all either — should just list everything, not crash.
        let hits =
            SearchRepository::search_filename(&db.conn, &[], &Filters::default(), 10).unwrap();
        assert_eq!(hits.len(), 2, "hits: {:?}", hits);
    }

    #[test]
    fn search_content_with_only_filters_dedupes_by_document() {
        let db = Db::open_in_memory().unwrap();
        db.conn
            .execute(
                "INSERT INTO documents (document_id, file_size, text_bytes, index_tier)
             VALUES ('doc1', 0, 0, 'FULL')",
                [],
            )
            .unwrap();
        // Same document reachable via two paths — must appear once, not twice.
        db.conn.execute(
            "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
             VALUES ('/a/규정.txt', 'doc1', '규정.txt', 'txt', '2026-08-10T00:00:00Z', '2026-08-10T00:00:00Z')",
            [],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO paths (path, document_id, filename, extension, modified_at, seen_at)
             VALUES ('/b/규정_사본.txt', 'doc1', '규정_사본.txt', 'txt', '2026-08-09T00:00:00Z', '2026-08-09T00:00:00Z')",
            [],
        ).unwrap();

        let hits = SearchRepository::search_content(&db.conn, "", &Filters::default(), 10).unwrap();
        assert_eq!(hits.len(), 1, "hits: {:?}", hits);

        let wrong_extension = Filters {
            extension: Some("pdf".to_string()),
            ..Filters::default()
        };
        let hits = SearchRepository::search_content(&db.conn, "", &wrong_extension, 10).unwrap();
        assert!(hits.is_empty(), "hits: {:?}", hits);
    }
}
