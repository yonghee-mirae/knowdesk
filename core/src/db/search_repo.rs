//! `filename_fts` / `content_fts` 색인·조회.
//!
//! bm25 컬럼 가중치: `body`가 가장 우세하고, `morph`(bigram, 항상 채워지는 기본
//! 토크나이저)와 `morph_kiwi`(Kiwi, 가능할 때만 채워지는 보조 토크나이저)가 보조
//! 신호로 더해진다. Kiwi가 bigram보다 정밀하므로 weight를 더 높게 둔다 — 세 값
//! 모두 실측 전 잠정치이며 조정 가능하다. 필터(`ext`/`path`/`tier`/`drm`/`modified`)는
//! `documents`/`paths`와 조인해 SQL 단에서 적용한다.

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
    /// content 모드에서만 채워진다 — 검색어 확장(`morph_kiwi`) 매칭 여부를
    /// 나중에 판별하기 위해 필요하다 (`search::service`).
    pub document_id: Option<String>,
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

    /// 경로 하나의 파일명 색인을 지운다 (파일 삭제/이동 감시 시 사용).
    pub fn remove_filename(conn: &Connection, path: &str) -> rusqlite::Result<()> {
        conn.execute("DELETE FROM filename_fts WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// 문서 하나의 본문 색인을 지운다 (그 문서를 참조하는 경로가 하나도 안 남았을
    /// 때 `DocumentRepository::remove_path`가 정리 차원에서 호출한다).
    pub fn remove_content(conn: &Connection, document_id: &str) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM content_fts WHERE document_id = ?1",
            params![document_id],
        )?;
        Ok(())
    }

    /// 본문 색인. 문서는 내용 기준(document_id)이라 기존 행을 지우고 다시 넣는다.
    /// `morph_kiwi`는 Kiwi를 못 쓰는 환경이면 빈 문자열이다.
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
                    bm25(content_fts, ?, ?, ?),
                    content_fts.document_id
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
        // 중복 제거 전이므로 경로 수만큼 더 넉넉히 가져온다.
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
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut seen = std::collections::HashSet::new();
        let deduped = rows
            .into_iter()
            .filter(|row| seen.insert(row.document_id.clone()))
            .take(limit as usize)
            .collect();
        // body(0번 컬럼) 스니펫에 강조 표시가 없는 히트(morph/morph_kiwi로만 걸린
        // 경우, 예: "레이아웃과"에서 "레이아웃")를 원문에서 직접 찾아 강조하는
        // 보강은 `search::service`가 한다 — 저장된 원문(`document_bodies`)이
        // 필요해서다.
        Ok(deduped)
    }

    /// 이 문서가 (검색어 확장 없이) `literal_expr` 그대로에 매칭되는지 확인한다.
    /// 검색어 확장으로 찾은 히트가 "정확 일치"인지 "형태소 분석에 의한 매칭"인지
    /// 구분하는 데 쓴다 (`search::service`).
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
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
