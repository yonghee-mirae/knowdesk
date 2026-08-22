//! Query Parser (`docs/05_Search_Language_v1.md`).
//!
//! Keyword/Phrase/AND/OR/NOT/Prefix are already supported as-is by FTS5 MATCH
//! syntax, so the job here is to strip out filter tokens
//! (`ext:`/`path:`/`tier:`/`drm:`/`modified>`/`modified<`) and pass the rest
//! straight through as an FTS5 MATCH string.

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
    /// The literal search-term tokens with filters stripped out. `match_expr`
    /// is just these joined with spaces — kept separate because query
    /// expansion (`search::service`) needs to rework them token by token.
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

/// Determines whether `term` is a "plain search term" that's safe to run
/// through morphological analysis. Phrases ("..."), prefix search (발행*),
/// and AND/OR/NOT operators must be left as FTS5 syntax, so they're excluded
/// from analysis.
pub fn is_plain_keyword(term: &str) -> bool {
    !term.starts_with('"') && !term.ends_with('*') && !matches!(term, "AND" | "OR" | "NOT")
}

/// Passing a plain search term containing characters that have special
/// meaning in FTS5 syntax (period, hyphen, slash, etc.) — like "3.2%",
/// "2026-08-21", or "P/E" — straight through causes the FTS5 parser to raise
/// a syntax error (confirmed in practice). Such terms are wrapped whole in a
/// phrase ("...") so they're treated as literals. Phrases/AND·OR·NOT/prefix
/// search (`발행*`) are left untouched since they must stay as FTS5 syntax.
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

/// Whether this is a form FTS5 safely accepts without quoting (consists only
/// of letters/digits/underscore).
fn is_safe_bareword(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Splits on whitespace, but keeps `"..."` phrases as a single token.
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
        // Passing a search term containing characters that have special
        // meaning in FTS5 syntax (period/percent/hyphen/slash) straight
        // through causes the parser to raise a syntax error (confirmed in
        // practice). It must be wrapped whole in a phrase so it's treated as
        // a literal.
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
