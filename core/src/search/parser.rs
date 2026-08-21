//! Query Parser (`docs/05_Search_Language_v1.md`).
//!
//! Keyword/Phrase/AND/OR/NOT/Prefix는 FTS5 MATCH 문법이 이미 그대로 지원하므로,
//! 여기서 할 일은 필터 토큰(`ext:`/`path:`/`tier:`/`drm:`/`modified>`/`modified<`)을
//! 걷어내고 나머지를 FTS5 MATCH 문자열로 그대로 넘기는 것이다.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub extension: Option<String>,
    pub path_contains: Option<String>,
    pub tier: Option<String>,
    pub drm: Option<bool>,
    pub modified_after: Option<String>,
    pub modified_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    pub match_expr: String,
    /// 필터를 걷어낸 리터럴 검색어 토큰들. `match_expr`은 이걸 공백으로 합친 것과
    /// 같다 — 검색어 확장(`search::service`)이 토큰 단위로 다시 손봐야 해서 따로 둔다.
    pub terms: Vec<String>,
    pub filters: Filters,
}

pub fn parse(input: &str) -> ParsedQuery {
    let mut filters = Filters::default();
    let mut terms = Vec::new();

    for token in tokenize(input) {
        if let Some(rest) = token.strip_prefix("ext:") {
            filters.extension = Some(rest.trim_start_matches('.').to_lowercase());
        } else if let Some(rest) = token.strip_prefix("path:") {
            filters.path_contains = Some(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("tier:") {
            filters.tier = Some(rest.to_uppercase());
        } else if let Some(rest) = token.strip_prefix("drm:") {
            filters.drm = Some(rest.eq_ignore_ascii_case("true"));
        } else if let Some(rest) = token.strip_prefix("modified>") {
            filters.modified_after = Some(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("modified<") {
            filters.modified_before = Some(rest.to_string());
        } else {
            terms.push(sanitize_term(&token));
        }
    }

    ParsedQuery {
        match_expr: terms.join(" "),
        terms,
        filters,
    }
}

/// `term`이 형태소 분석을 적용해도 안전한 "평범한 검색어"인지 판별한다.
/// 문구("..."), 접두 검색(발행*), AND/OR/NOT 연산자는 FTS5 문법 그대로 둬야 하므로
/// 분석 대상에서 제외한다.
pub fn is_plain_keyword(term: &str) -> bool {
    !term.starts_with('"') && !term.ends_with('*') && !matches!(term, "AND" | "OR" | "NOT")
}

/// "3.2%"·"2026-08-21"·"P/E"처럼 FTS5 문법에서 특수 의미를 갖는 문자(마침표,
/// 하이픈, 슬래시 등)가 든 평범한 검색어를 그대로 넘기면 FTS5 파서가 구문 오류를
/// 낸다(실제로 확인됨). 그런 검색어는 통째로 구문("...")으로 감싸 리터럴 취급되게
/// 한다. 문구/AND·OR·NOT/접두 검색(`발행*`)은 FTS5 문법 그대로 둬야 하므로 손대지
/// 않는다.
fn sanitize_term(term: &str) -> String {
    if term.starts_with('"') || matches!(term, "AND" | "OR" | "NOT") {
        return term.to_string();
    }
    if let Some(prefix) = term.strip_suffix('*') {
        if is_safe_bareword(prefix) {
            return term.to_string();
        }
    }
    if is_safe_bareword(term) {
        term.to_string()
    } else {
        format!("\"{}\"", term.replace('"', "\"\""))
    }
}

/// FTS5가 인용 없이 안전하게 받아들이는 형태(문자/숫자/밑줄로만 구성)인지.
fn is_safe_bareword(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// 공백으로 나누되, `"..."` 구문은 하나의 토큰으로 유지한다.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in input.chars() {
        if ch == '"' {
            in_quotes = !in_quotes;
            current.push(ch);
        } else if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_plain_keywords() {
        let parsed = parse("채권 발행");
        assert_eq!(parsed.match_expr, "채권 발행");
        assert_eq!(parsed.filters, Filters::default());
    }

    #[test]
    fn keeps_phrase_quotes_intact() {
        let parsed = parse("\"채권 발행\"");
        assert_eq!(parsed.match_expr, "\"채권 발행\"");
    }

    #[test]
    fn passes_through_and_or_not_prefix() {
        assert_eq!(parse("채권 AND 발행").match_expr, "채권 AND 발행");
        assert_eq!(parse("채권 OR 회사채").match_expr, "채권 OR 회사채");
        assert_eq!(parse("채권 NOT 국채").match_expr, "채권 NOT 국채");
        assert_eq!(parse("발행*").match_expr, "발행*");
    }

    #[test]
    fn quotes_terms_with_fts5_special_characters() {
        // FTS5 문법에서 특수 의미를 갖는 문자(마침표/퍼센트/하이픈/슬래시)가 든
        // 검색어를 그대로 넘기면 파서가 구문 오류를 낸다(실제로 확인됨). 통째로
        // 구문으로 감싸 리터럴 취급되게 해야 한다.
        assert_eq!(parse("3.2%").match_expr, "\"3.2%\"");
        assert_eq!(parse("2026-08-21").match_expr, "\"2026-08-21\"");
        assert_eq!(parse("P/E").match_expr, "\"P/E\"");
    }

    #[test]
    fn keeps_plain_english_and_korean_barewords_unquoted() {
        assert_eq!(parse("KOSPI GDP").match_expr, "KOSPI GDP");
        assert_eq!(parse("채권").match_expr, "채권");
    }

    #[test]
    fn extracts_filters() {
        let parsed = parse(
            "채권 ext:pdf path:리서치 tier:full drm:true modified>2026-01-01 modified<2026-08-01",
        );
        assert_eq!(parsed.match_expr, "채권");
        assert_eq!(parsed.filters.extension.as_deref(), Some("pdf"));
        assert_eq!(parsed.filters.path_contains.as_deref(), Some("리서치"));
        assert_eq!(parsed.filters.tier.as_deref(), Some("FULL"));
        assert_eq!(parsed.filters.drm, Some(true));
        assert_eq!(parsed.filters.modified_after.as_deref(), Some("2026-01-01"));
        assert_eq!(
            parsed.filters.modified_before.as_deref(),
            Some("2026-08-01")
        );
    }
}
