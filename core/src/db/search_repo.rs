//! `filename_fts` / `content_fts` 색인·조회.
//!
//! bm25 컬럼 가중치는 `docs/D2_검색_랭킹_옵션_비교.md` 1번 항목의 추천안(body 우세,
//! `body:1.0 / morph:0.3`)을 따른다. 필터(`ext`/`path`/`tier`/`drm`/`modified`)는
//! `documents`/`paths`와 조인해 SQL 단에서 적용한다.

use rusqlite::types::ToSql;
use rusqlite::{params, Connection};

use crate::search::parser::Filters;

pub const CONTENT_BODY_WEIGHT: f64 = 1.0;
pub const CONTENT_MORPH_WEIGHT: f64 = 0.3;

#[derive(Debug, Clone)]
pub struct SearchRow {
    pub path: String,
    pub filename: String,
    pub snippet: Option<String>,
    pub rank: f64,
}

pub struct SearchRepository;

impl SearchRepository {
    /// 파일명 색인. 같은 경로가 이미 있으면 지우고 다시 넣는다 (fts5는 UNIQUE 제약이 없음).
    pub fn index_filename(conn: &Connection, path: &str, filename: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM filename_fts WHERE path = ?1", params![path])?;
        conn.execute(
            "INSERT INTO filename_fts (filename, path) VALUES (?1, ?2)",
            params![filename, path],
        )?;
        Ok(())
    }

    /// 본문 색인. 문서는 내용 기준(document_id)이라 기존 행을 지우고 다시 넣는다.
    pub fn index_content(
        conn: &Connection,
        document_id: &str,
        body: &str,
        morph: &str,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM content_fts WHERE document_id = ?1",
            params![document_id],
        )?;
        conn.execute(
            "INSERT INTO content_fts (body, morph, document_id) VALUES (?1, ?2, ?3)",
            params![body, morph, document_id],
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
            "SELECT f.path, f.filename, NULL, bm25(filename_fts)
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
        // bm25()/snippet()는 fts5 MATCH 커서 컨텍스트가 있어야 호출 가능하다.
        // GROUP BY를 쓰면 SQLite가 정렬/집계 단계를 거치며 그 컨텍스트를 잃어
        // "unable to use function in the requested context" 에러가 나므로,
        // 문서당 여러 경로 중복 제거는 GROUP BY 대신 Rust 쪽에서 처리한다.
        let mut sql = String::from(
            "SELECT p.path, p.filename,
                    snippet(content_fts, 0, '>>', '<<', '...', 12),
                    bm25(content_fts, ?, ?),
                    content_fts.document_id
             FROM content_fts
             JOIN documents d ON d.document_id = content_fts.document_id
             JOIN paths p ON p.document_id = d.document_id
             WHERE content_fts MATCH ?",
        );
        let mut sql_params: Vec<Box<dyn ToSql>> = vec![
            Box::new(CONTENT_BODY_WEIGHT),
            Box::new(CONTENT_MORPH_WEIGHT),
            Box::new(match_expr.to_string()),
        ];
        push_filter_clauses(&mut sql, &mut sql_params, filters);
        sql.push_str(" ORDER BY 4 LIMIT ?");
        // 중복 제거 전이므로 경로 수만큼 더 넉넉히 가져온다.
        sql_params.push(Box::new(limit.saturating_mul(5).max(limit)));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn ToSql> = sql_params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    SearchRow {
                        path: row.get(0)?,
                        filename: row.get(1)?,
                        snippet: row.get(2)?,
                        rank: row.get(3)?,
                    },
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen = std::collections::HashSet::new();
        let deduped = rows
            .into_iter()
            .filter(|(_, document_id)| seen.insert(document_id.clone()))
            .map(|(row, _)| row)
            .take(limit as usize)
            .collect();
        Ok(deduped)
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
