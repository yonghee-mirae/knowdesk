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
    /// Secondary tokenizer — if present, expands content-mode search terms
    /// via morphological analysis. Filename mode is indexed without
    /// morphological analysis in the first place, so it never uses this
    /// field.
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
        // Try the original search term first; if that's not found, also try
        // the stem Kiwi actually used for matching — e.g. if the query was
        // "수행함" but the source text only has "수행한다", the stem "수행"
        // can still be found and highlighted in the source text.
        let mut needles = literal_needles(parsed);
        needles.extend(analyzed_forms);

        let mut hits = Vec::with_capacity(rows.len());
        for mut row in rows {
            // If we didn't expand, only "exact match" exists in the first
            // place — no need to re-verify.
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

            // No highlight in the body-column (column 0) snippet means it
            // matched only via morph/morph_kiwi (e.g. "레이아웃" within
            // "레이아웃과" — with a particle attached, that token isn't in
            // the body column). We reinforce this by looking up the search
            // term directly in the stored source text and highlighting it:
            //   1. Try a literal lookup (the search term as-is, or the
            //      Kiwi-analyzed stem).
            //   2. If that still fails (as with irregular conjugations where
            //      the surface form itself differs — e.g. "지었다" doesn't
            //      contain the letters "짓"), and Kiwi can tell us the actual
            //      position of the word segment that morpheme belongs to in
            //      the source text, highlight that span.
            // If neither works, leave the source text unhighlighted — better
            // than a bare token list.
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

            // FTS5's snippet() decides the highlight range based on its own
            // tokenizer, so a character that isn't part of a token — like
            // "%" — can be left outside the highlight even though it's part
            // of the search term (e.g. searching "3.2%" gives >>3.2<<%). If
            // what immediately follows the highlight continues into the rest
            // of the search term, widen the highlight to cover it.
            if !needles.is_empty() {
                row.snippet = row.snippet.map(|s| widen_highlights(&s, &needles));
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

/// Analyzes plain words in the search term with Kiwi and expands them into
/// `(literal OR morph_kiwi:(analyzed morphemes...))`. Phrases/operators/prefix
/// search are left as-is. If the analysis result doesn't differ from the
/// literal (e.g. searching a common noun), the literal is kept as-is — bigram
/// is not used for query analysis (see `11_Implementation_Plan.md`). Returns
/// both the expanded query expression and the list of analyzed morphemes
/// actually used in matching (for reinforcing snippet highlights, see
/// `highlight_literal_match`).
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

/// Builds the list of search terms to look up literally in the source text.
/// AND/OR/NOT are excluded since they're operators, not search terms. For
/// phrases the surrounding quotes are stripped, and for prefix search the
/// trailing `*` is stripped, leaving just the plain text.
fn literal_needles(parsed: &ParsedQuery) -> Vec<String> {
    parsed
        .terms
        .iter()
        .filter(|term| !matches!(term.as_str(), "AND" | "OR" | "NOT"))
        .map(|term| term.trim_matches('"').trim_end_matches('*').to_string())
        .filter(|term| !term.is_empty())
        .collect()
}

/// Case-insensitively finds one of `needles` in the stored source text
/// (`body`) and returns its character position (start, length). Returns
/// `None` if none of them exist literally — the caller then tries again with
/// `Tokenizer::locate`.
fn find_literal_span(body: &str, needles: &[String]) -> Option<(usize, usize)> {
    let body_chars: Vec<char> = body.chars().collect();
    needles.iter().find_map(|needle| {
        let needle_chars: Vec<char> = needle.chars().collect();
        find_ignore_ascii_case(&body_chars, &needle_chars).map(|pos| (pos, needle_chars.len()))
    })
}

/// Wraps the character-based span `[start, start+len)` of `body` in `>>...<<`
/// and attaches surrounding context to build a snippet. Adds `...` on the
/// side that got truncated.
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

/// If the characters right after a highlight (`>>...<<`) span continue into
/// the rest of a `needle` for which the highlighted text is a prefix, widen
/// the highlight to cover them. Because FTS5's snippet() decides the
/// highlight range based on its own tokenizer, a character that isn't part
/// of a token (%) can be left outside the highlight even though it's part of
/// the search term, as with "3.2%" (`>>3.2<<%`) — this fixes such cases so
/// they look natural to a human reader.
fn widen_highlights(snippet: &str, needles: &[String]) -> String {
    let needle_chars: Vec<Vec<char>> = needles.iter().map(|n| n.chars().collect()).collect();
    let chars: Vec<char> = snippet.chars().collect();

    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&['>', '>']) {
            let content_start = i + 2;
            let Some(gap) = chars[content_start..]
                .windows(2)
                .position(|w| w == ['<', '<'])
            else {
                out.extend(&chars[i..]);
                break;
            };
            let content_end = content_start + gap;
            let highlighted = &chars[content_start..content_end];
            let after = &chars[content_end + 2..];

            let extra = needle_chars
                .iter()
                .filter_map(|needle| {
                    if needle.len() <= highlighted.len() {
                        return None;
                    }
                    let prefix_matches = highlighted
                        .iter()
                        .zip(needle)
                        .all(|(h, n)| h.eq_ignore_ascii_case(n));
                    if !prefix_matches {
                        return None;
                    }
                    let remainder = &needle[highlighted.len()..];
                    if after.len() < remainder.len() {
                        return None;
                    }
                    let suffix_matches = after[..remainder.len()]
                        .iter()
                        .zip(remainder)
                        .all(|(a, r)| a.eq_ignore_ascii_case(r));
                    suffix_matches.then_some(remainder.len())
                })
                .max()
                .unwrap_or(0);

            out.push_str(">>");
            out.extend(highlighted);
            out.extend(&after[..extra]);
            out.push_str("<<");
            i = content_end + 2 + extra;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Case-insensitively finds the position of the first character (character
/// index, not byte) of `needle` within `haystack`. Since Hangul has no case,
/// this is equivalent to plain equality there, and it only has real effect on
/// Latin text (e.g. finding "kospi" in the source text with the search term
/// "KOSPI").
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
