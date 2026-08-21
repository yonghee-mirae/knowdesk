use rusqlite::Connection;

use super::parser::{is_plain_keyword, parse, ParsedQuery};
use super::{
    MatchKind, SearchError, SearchHit, SearchMode, SearchRequest, SearchResult,
    SearchService as SearchServiceTrait,
};
use crate::db::search_repo::{SearchRepository, SearchRow};
use crate::db::store::{DocumentStore, SqliteDocumentStore};
use crate::nlp::Tokenizer;

pub struct SqliteSearchService<'a> {
    pub conn: &'a Connection,
    /// 보조 토크나이저 — 있으면 content 모드 검색어를 형태소 분석해서 확장한다.
    /// filename 모드는 애초에 형태소 분석 없이 색인되므로 이 필드를 쓰지 않는다.
    pub kiwi: Option<&'a dyn Tokenizer>,
}

impl<'a> SearchServiceTrait for SqliteSearchService<'a> {
    fn search(&self, request: &SearchRequest) -> Result<SearchResult, SearchError> {
        let parsed = parse(&request.query);

        let hits = match request.mode {
            SearchMode::Filename => SearchRepository::search_filename(
                self.conn,
                &parsed.match_expr,
                &parsed.filters,
                request.limit,
            )?
            .into_iter()
            .map(|row| to_hit(row, MatchKind::Exact))
            .collect(),
            SearchMode::Content => self.search_content(&parsed, request.limit)?,
        };

        Ok(SearchResult { hits })
    }
}

impl<'a> SqliteSearchService<'a> {
    fn search_content(
        &self,
        parsed: &ParsedQuery,
        limit: i64,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let (query_expr, analyzed_forms) = match self.kiwi {
            Some(kiwi) => expand_with_kiwi(parsed, kiwi),
            None => (parsed.match_expr.clone(), Vec::new()),
        };
        let expanded = query_expr != parsed.match_expr;

        let rows =
            SearchRepository::search_content(self.conn, &query_expr, &parsed.filters, limit)?;
        // 원본 검색어를 먼저 찾아보고, 못 찾으면 Kiwi가 실제 매칭에 쓴 어간도
        // 시도한다 — 예: "수행함"으로 검색했지만 원문엔 "수행한다"만 있는 경우,
        // 어간 "수행"으로는 원문에서 찾아 강조할 수 있다.
        let mut needles = literal_needles(parsed);
        needles.extend(analyzed_forms);

        let mut hits = Vec::with_capacity(rows.len());
        for mut row in rows {
            // 확장 안 했으면 애초에 "정확 일치"만 존재한다 — 굳이 재확인하지 않는다.
            let match_kind = if !expanded {
                MatchKind::Exact
            } else {
                match &row.document_id {
                    Some(document_id)
                        if SearchRepository::document_matches_content(
                            self.conn,
                            document_id,
                            &parsed.match_expr,
                        )? =>
                    {
                        MatchKind::Exact
                    }
                    Some(_) => MatchKind::Morphological,
                    None => MatchKind::Exact,
                }
            };

            // body(0번 컬럼) 스니펫에 강조 표시가 없다는 건 morph/morph_kiwi로만
            // 걸렸다는 뜻이다(예: "레이아웃과"에서 "레이아웃" — 조사가 붙어서
            // body 컬럼엔 그 토큰이 없다). 저장된 원문에서 검색어를 직접 찾아
            // 강조해 보강한다:
            //   1. 리터럴로 찾아본다 (검색어 그대로, 또는 Kiwi 분석 어간).
            //   2. 그래도 못 찾으면(불규칙 활용형처럼 표면형 자체가 다른 경우,
            //      예: "지었다"엔 "짓"이라는 글자가 없음) Kiwi가 원문에서 그
            //      형태소가 속한 어절의 실제 위치를 알려주면 그 구간을 강조한다.
            // 둘 다 안 되면 강조 없는 원문 그대로 둔다 — 그게 토큰 나열보다 낫다.
            let has_highlight = row.snippet.as_deref().is_some_and(|s| s.contains(">>"));
            if !has_highlight && !needles.is_empty() {
                if let Some(document_id) = &row.document_id {
                    let store = SqliteDocumentStore { conn: self.conn };
                    if let Some(body) = store.get_body(document_id)? {
                        let span = find_literal_span(&body, &needles)
                            .or_else(|| self.kiwi.and_then(|k| k.locate(&body, &needles)));
                        if let Some((start, len)) = span {
                            row.snippet = Some(build_snippet(&body, start, len));
                        }
                    }
                }
            }

            hits.push(to_hit(row, match_kind));
        }

        Ok(hits)
    }
}

fn to_hit(row: SearchRow, match_kind: MatchKind) -> SearchHit {
    SearchHit {
        path: row.path,
        filename: row.filename,
        snippet: row.snippet,
        rank: row.rank,
        match_kind,
    }
}

/// 검색어의 평범한 단어들을 Kiwi로 분석해 `(원문 OR morph_kiwi:(분석 형태소...))`로
/// 확장한다. 문구/연산자/접두 검색은 그대로 둔다. 분석 결과가 원문과 다르지 않으면
/// (흔한 명사 검색 등) 원문 그대로 둔다 — bigram은 검색어 분석에 쓰지 않는다
/// (`11_Implementation_Plan.md` 참조). 확장된 질의문과, 실제로 매칭에 쓰인 분석
/// 형태소 목록(스니펫 강조 보강용, `highlight_literal_match` 참조)을 같이 반환한다.
fn expand_with_kiwi(parsed: &ParsedQuery, kiwi: &dyn Tokenizer) -> (String, Vec<String>) {
    let mut analyzed_forms = Vec::new();
    let query_expr = parsed
        .terms
        .iter()
        .map(|term| {
            if !is_plain_keyword(term) {
                return term.clone();
            }

            let mut distinct = Vec::new();
            for token in kiwi.tokenize(term) {
                if token.0 != *term && !distinct.contains(&token.0) {
                    distinct.push(token.0);
                }
            }

            if distinct.is_empty() {
                term.clone()
            } else {
                let expr = format!("({term} OR morph_kiwi:({}))", distinct.join(" OR "));
                analyzed_forms.extend(distinct);
                expr
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (query_expr, analyzed_forms)
}

/// 원문에서 그대로 찾아볼 검색어 목록을 만든다. AND/OR/NOT은 검색어가 아니라
/// 연산자라 뺀다. 문구는 감싼 인용부호를, 접두 검색은 끝의 `*`를 벗겨서
/// 순수 텍스트만 남긴다.
fn literal_needles(parsed: &ParsedQuery) -> Vec<String> {
    parsed
        .terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "AND" | "OR" | "NOT"))
        .map(|term| term.trim_matches('"').trim_end_matches('*').to_string())
        .filter(|term| !term.is_empty())
        .collect()
}

/// 저장된 원문(`body`)에서 `needles` 중 하나를 대소문자 무관으로 찾아, 글자 위치
/// (시작, 길이)를 돌려준다. 어느 것도 리터럴로 없으면 `None` — 그러면 호출부가
/// `Tokenizer::locate`로 한 번 더 시도한다.
fn find_literal_span(body: &str, needles: &[String]) -> Option<(usize, usize)> {
    let body_chars: Vec<char> = body.chars().collect();
    needles.iter().find_map(|needle| {
        let needle_chars: Vec<char> = needle.chars().collect();
        find_ignore_ascii_case(&body_chars, &needle_chars).map(|pos| (pos, needle_chars.len()))
    })
}

/// `body`의 글자 단위 구간 `[start, start+len)`을 `>>...<<`로 감싸고, 앞뒤로
/// 문맥을 붙여 스니펫을 만든다. 잘려나간 쪽에는 `...`을 붙인다.
fn build_snippet(body: &str, start: usize, len: usize) -> String {
    const CONTEXT_CHARS: usize = 40;

    let body_chars: Vec<char> = body.chars().collect();
    let before = start.saturating_sub(CONTEXT_CHARS);
    let after = (start + len + CONTEXT_CHARS).min(body_chars.len());

    let mut snippet = String::new();
    if before > 0 {
        snippet.push_str("...");
    }
    snippet.extend(&body_chars[before..start]);
    snippet.push_str(">>");
    snippet.extend(&body_chars[start..start + len]);
    snippet.push_str("<<");
    snippet.extend(&body_chars[start + len..after]);
    if after < body_chars.len() {
        snippet.push_str("...");
    }
    snippet
}

/// `needle`이 `haystack` 안에 있는 첫 글자 위치(문자 단위 인덱스, 바이트 아님)를
/// 대소문자 무관으로 찾는다. 한글엔 대소문자가 없어 순수 동등 비교와 같고,
/// 영문에만 실질적으로 영향을 준다(예: "KOSPI" 검색어로 "kospi" 원문을 찾음).
fn find_ignore_ascii_case(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    (0..=haystack.len() - needle.len()).find(|&start| {
        haystack[start..start + needle.len()]
            .iter()
            .zip(needle)
            .all(|(h, n)| h.eq_ignore_ascii_case(n))
    })
}
