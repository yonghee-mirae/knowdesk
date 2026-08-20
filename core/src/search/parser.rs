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
            terms.push(token);
        }
    }

    ParsedQuery {
        match_expr: terms.join(" "),
        filters,
    }
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
