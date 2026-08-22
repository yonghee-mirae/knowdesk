//! Indexing and querying for `filename_fts` / `content_fts`.
//!
//! bm25 column weights: `body` dominates, with `morph` (bigram, the default tokenizer,
//! always populated) and `morph_kiwi` (Kiwi, the secondary tokenizer, populated only when
//! available) added as secondary signals. Kiwi is more precise than bigram, so it gets a
//! higher weight — all three values are provisional pending real measurement and subject to
//! tuning. Filters (`ext`/`path`/`tier`/`drm`/`modified`) are applied at the SQL level by
//! joining against `documents`/`paths`.

use rusqlite::types::ToSql;
use rusqlite::{params, Connection, OptionalExtension};

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
    /// Index a filename. If the same path already exists, delete and re-insert it
    /// (fts5 has no UNIQUE constraint).
    pub fn index_filename(conn: &Connection, path: &str, filename: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM filename_fts WHERE path = ?1", params![path])?;
        conn.execute(
            "INSERT INTO filename_fts (filename, path) VALUES (?1, ?2)",
            params![filename, path],
        )?;
        Ok(())
    }

    /// Removes the filename index entry for a single path (used when watching for file
    /// deletion/move).
    pub fn remove_filename(conn: &Connection, path: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM filename_fts WHERE path = ?1", params![path])?;
        Ok(())
    }

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

    pub fn search_filename(
        conn: &Connection,
        match_expr: &str,
        filters: &Filters,
        limit: i64,
    ) -> rusqlite::Result<Vec<SearchRow>> {
        let mut sql = String::from(
            "SELECT f.path, f.filename, NULL, bm25(filename_fts),
                    p.extension, p.modified_at, d.index_tier
             FROM filename_fts f
             JOIN paths p ON p.path = f.path
             JOIN documents d ON d.document_id = p.document_id
             WHERE filename_fts MATCH ?",
        );
        let mut sql_params: Vec<Box<dyn ToSql>> = vec![Box::new(match_expr.to_string())];
        push_filter_clauses(&mut sql, &mut sql_params, filters);
        sql.push_str(" ORDER BY 4 LIMIT ?");
        sql_params.push(Box::new(limit));

        run_search_query(conn, &sql, &sql_params)
    }

    pub fn search_content(
        conn: &Connection,
        match_expr: &str,
        filters: &Filters,
        limit: i64,
    ) -> rusqlite::Result<Vec<SearchRow>> {
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

        let mut seen = std::collections::HashSet::new();
        let deduped = rows
            .into_iter()
            .filter(|row| seen.insert(row.document_id.clone()))
            .take(limit as usize)
            .collect();
        // Enhancing hits whose body (column 0) snippet has no highlight (i.e. matched only
        // via morph/morph_kiwi, e.g. "레이아웃" within "레이아웃과") by finding and
        // highlighting the term directly in the original text is done by `search::service`,
        // since it needs the stored original text (`document_bodies`).
        Ok(deduped)
    }

    /// Checks whether this document matches `literal_expr` as-is (without query expansion).
    /// Used to distinguish whether a hit found via query expansion is an "exact match" or a
    /// "match via morphological analysis" (`search::service`).
    pub fn document_matches_content(
        conn: &Connection,
        document_id: &str,
        literal_expr: &str,
    ) -> rusqlite::Result<bool> {
        let matched: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM content_fts
                 WHERE content_fts MATCH ?1 AND document_id = ?2 LIMIT 1",
                params![literal_expr, document_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(matched.is_some())
    }
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
    if let Some(tier) = &filters.tier {
        sql.push_str(" AND d.index_tier = ?");
        params.push(Box::new(tier.clone()));
    }
    if let Some(drm) = filters.drm {
        sql.push_str(if drm {
            " AND d.drm_status = 'DRM'"
        } else {
            " AND (d.drm_status IS NULL OR d.drm_status != 'DRM')"
        });
    }
    if let Some(after) = &filters.modified_after {
        sql.push_str(" AND p.modified_at > ?");
        params.push(Box::new(after.clone()));
    }
    if let Some(before) = &filters.modified_before {
        sql.push_str(" AND p.modified_at < ?");
        params.push(Box::new(before.clone()));
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
