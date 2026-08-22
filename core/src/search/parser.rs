//! Query Parser (`docs/05_Search_Language_v1.md`).
//!
//! Keyword/Phrase/AND/OR/NOT/Prefix are already supported as-is by FTS5 MATCH
//! syntax, so the job here is to strip out filter tokens
//! (`ext:`/`path:`/`tier:`/`drm:`/`modified>`/`modified<`/`modified=`) and
//! pass the rest straight through as an FTS5 MATCH string.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub extension: Option<String>,
    pub path_contains: Option<String>,
    pub tier: Option<String>,
    pub drm: Option<bool>,
    pub modified_after: Option<String>,
    pub modified_before: Option<String>,
    /// Exact calendar-day match, e.g. `modified=2026-08-10`. `paths.modified_at` is a
    /// full RFC3339 timestamp, so this is compared by calendar day
    /// (`push_filter_clauses` wraps both sides in SQLite's `date()`), not by exact
    /// string equality — otherwise it would never match anything.
    pub modified_on: Option<String>,
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
        } else if let Some(rest) = token.strip_prefix("modified=") {
            filters.modified_on = Some(rest.to_string());
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
/// AND/OR/NOT operators, and grouping parentheses must be left as FTS5
/// syntax, so they're excluded from analysis — wrapping a term like `(건물`
/// in `(term OR morph_kiwi:(...))` would nest the user's own grouping
/// paren inside ours and corrupt it.
pub fn is_plain_keyword(term: &str) -> bool {
    !term.starts_with('"')
        && !term.ends_with('*')
        && !term.contains('(')
        && !term.contains(')')
        && !matches!(term, "AND" | "OR" | "NOT")
}

/// Passing a plain search term containing characters that have special
/// meaning in FTS5 syntax (period, hyphen, slash, etc.) — like "3.2%",
/// "2026-08-21", or "P/E" — straight through causes the FTS5 parser to raise
/// a syntax error (confirmed in practice). Such terms are wrapped whole in a
/// phrase ("...") so they're treated as literals. Phrases/AND·OR·NOT/prefix
/// search (`발행*`) are left untouched since they must stay as FTS5 syntax.
///
/// `(`/`)` are also left unquoted rather than folded into that literal-phrase
/// escaping — they're FTS5's own grouping syntax (`(a OR b) AND c`), not
/// incidental punctuation. Quoting them used to silently defeat grouping: our
/// own tokenizer only splits on whitespace, so `(건물` stayed one token and
/// got wrapped as a literal phrase `"(건물"`; FTS5 then tokenizes phrase
/// content the same way it tokenizes indexed text, which strips the `(` as
/// punctuation — so the phrase silently degraded to matching bare `건물`, and
/// the grouping just vanished with no error (confirmed in practice: `(a OR b)
/// AND c` and `a OR b AND c` returned identical results). Leaving `(`/`)`
/// unquoted lets FTS5's own query-syntax lexer see them as real grouping
/// tokens when the final match string reaches it.
fn sanitize_term(term: &str) -> String {
    if term.starts_with('"') || matches!(term, "AND" | "OR" | "NOT") {
        return term.to_string();
    }
    if let Some(prefix) = term.strip_suffix('*') {
        if is_safe_bareword(prefix) {
            return term.to_string();
        }
    }
    if is_safe_bareword_or_grouping(term) {
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

/// Same as `is_safe_bareword`, but also allows `(`/`)` through unquoted so
/// FTS5's own grouping syntax survives (see `sanitize_term`).
fn is_safe_bareword_or_grouping(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '(' || c == ')')
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
    fn keeps_grouping_parens_unquoted() {
        // Real bug, confirmed against the actual FTS5 index: quoting `(건물` as a
        // literal phrase made FTS5's own tokenizer strip the `(` as punctuation,
        // silently degrading it to a bare `건물` match and defeating the grouping —
        // with no error, so it looked like ordinary (wrong) results rather than a
        // failure. Parens must reach FTS5 unquoted to work as real grouping syntax.
        assert_eq!(
            parse("(건물 OR 채권) AND 결의").match_expr,
            "(건물 OR 채권) AND 결의"
        );
    }

    #[test]
    fn excludes_grouping_parens_from_kiwi_analysis() {
        // A term containing a grouping paren must not be treated as a plain keyword —
        // wrapping it in `(term OR morph_kiwi:(...))` for query expansion would nest
        // the user's own grouping paren inside ours and corrupt the expression.
        assert!(!is_plain_keyword("(건물"));
        assert!(!is_plain_keyword("채권)"));
    }

    #[test]
    fn extracts_filters() {
        let parsed = parse(
            "채권 ext:pdf path:리서치 tier:full drm:true modified>2026-01-01 modified<2026-08-01 modified=2026-08-10",
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
        assert_eq!(parsed.filters.modified_on.as_deref(), Some("2026-08-10"));
    }
}
