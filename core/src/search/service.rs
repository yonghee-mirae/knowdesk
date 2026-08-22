use rusqlite::Connection;

use super::parser::{is_plain_keyword, parse, parse_filename, ParsedQuery};
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
        let hits = match request.mode {
            SearchMode::Filename => {
                let (needles, filters) = parse_filename(&request.query);
                SearchRepository::search_filename(self.conn, &needles, &filters, request.limit)?
                    .into_iter()
                    .map(|row| to_hit(row, MatchKind::Exact))
                    .collect()
            }
            SearchMode::Content => {
                let parsed = parse(&request.query);
                self.search_content(&parsed, request.limit)?
            }
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
            // How this row was actually found decides both the match-kind tag
            // and (if FTS5's own body-column snippet came up empty) how to
            // build a highlight by hand:
            //   1. FTS5 already highlighted the body-column snippet — the
            //      search term is a literal token in the source text as-is.
            //      Exact, nothing more to do.
            //   2. Otherwise, look for the term as a literal character
            //      substring in the stored source text ourselves. This also
            //      catches matches FTS5's tokenizer can't isolate for
            //      snippet() (e.g. a particle glued onto a noun with no
            //      space, or a bigram-only substring match) — still an exact,
            //      character-for-character match even though FTS5 can't
            //      highlight it directly.
            //   3. Only when even that fails, and Kiwi's morphological
            //      analysis is the only reason this row was found at all
            //      (the surface form literally differs from the search term —
            //      e.g. the ㅅ-irregular "지었다" for the stem "짓"), is it a
            //      morphological match.
            // Previously this was decided by whether the *query* needed Kiwi
            // expansion at all, which mislabeled exactly this case as "exact"
            // (a plain, unexpanded query like "짓" can still only be found via
            // morph_kiwi, never literally in the source) — confirmed by the
            // snippet already highlighting a span the query's own characters
            // don't literally contain.
            let has_highlight = row.snippet.as_deref().is_some_and(|s| s.contains(">>"));
            let mut match_kind = MatchKind::Exact;

            if !has_highlight && !needles.is_empty() {
                if let Some(document_id) = &row.document_id {
                    let store = SqliteDocumentStore { conn: self.conn };
                    if let Some(body) = store.get_body(document_id)? {
                        if let Some((start, len)) = find_literal_span(&body, &needles) {
                            row.snippet = Some(build_snippet(&body, start, len));
                        } else if let Some((start, len)) =
                            self.kiwi.and_then(|k| k.locate(&body, &needles))
                        {
                            match_kind = MatchKind::Morphological;
                            row.snippet = Some(build_snippet(&body, start, len));
                        }
                    }
                }
            }

            // A multi-term query (e.g. "채권 OR 규정") can have FTS5 natively
            // highlight some needles (literal body tokens) but not others
            // (only reachable via bigram/morph_kiwi, like "규정" embedded in
            // "규정한다") — `has_highlight` above only checks whether *any*
            // needle got a highlight, so a query like that would otherwise
            // highlight only the first term and silently leave the second
            // one bare even when it's sitting right there in the same
            // excerpt. Find and highlight any needle that's visible verbatim
            // in the snippet text itself but not yet marked.
            if !needles.is_empty() {
                row.snippet = row.snippet.map(|s| highlight_missing_needles(&s, &needles));
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
        extension: row.extension,
        modified_at: row.modified_at,
        index_tier: row.index_tier,
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

/// Finds any `needle` that appears literally (case-insensitively) in
/// `snippet`'s plain text but isn't already inside a `>>...<<` span, and
/// highlights it too. Only looks within this snippet excerpt, not the whole
/// document — a needle that only occurs elsewhere in the source text is left
/// alone, since a single snippet window can't show every match location at
/// once.
fn highlight_missing_needles(snippet: &str, needles: &[String]) -> String {
    // Strip the existing >>/<< markers, recording which plain-text positions
    // were inside one, so a needle overlapping an existing highlight isn't
    // re-marked (and doesn't corrupt the marker pairing).
    let chars: Vec<char> = snippet.chars().collect();
    let mut plain: Vec<char> = Vec::with_capacity(chars.len());
    let mut highlighted: Vec<bool> = Vec::with_capacity(chars.len());
    let mut in_mark = false;
    let mut i = 0;
    while i < chars.len() {
        if chars[i..].starts_with(&['>', '>']) {
            in_mark = true;
            i += 2;
        } else if chars[i..].starts_with(&['<', '<']) {
            in_mark = false;
            i += 2;
        } else {
            plain.push(chars[i]);
            highlighted.push(in_mark);
            i += 1;
        }
    }

    for needle in needles {
        let needle_chars: Vec<char> = needle.chars().collect();
        if needle_chars.is_empty() {
            continue;
        }
        let mut search_from = 0;
        while let Some(offset) = find_ignore_ascii_case(&plain[search_from..], &needle_chars) {
            let start = search_from + offset;
            let end = start + needle_chars.len();
            if !highlighted[start..end].iter().any(|&h| h) {
                highlighted[start..end].iter_mut().for_each(|h| *h = true);
            }
            search_from = start + 1;
        }
    }

    let mut out = String::new();
    let mut idx = 0;
    while idx < plain.len() {
        if highlighted[idx] {
            out.push_str(">>");
            while idx < plain.len() && highlighted[idx] {
                out.push(plain[idx]);
                idx += 1;
            }
            out.push_str("<<");
        } else {
            out.push(plain[idx]);
            idx += 1;
        }
    }
    out
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
