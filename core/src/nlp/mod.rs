//! `Tokenizer` — morphological analyzer abstraction (`docs/08_API_Contracts.md`).
//! MVP starts with `BigramTokenizer`, and Phase B replaces it with `KiwiTokenizer`.

pub mod bigram;
pub mod kiwi;
pub mod synonym;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub String);

pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;

    /// Finds a morpheme in `text` whose form matches one of `forms`, and returns the
    /// character span (start, length — in the original text, character units) of the
    /// whole word segment (eojeol) that morpheme belongs to.
    /// This is for highlighting the exact span in the original text even when the
    /// surface form differs from the analyzed form due to conjugation (e.g. "지었다" (built)
    /// → stem "짓" (to build)) (`search::service`). The default implementation returns
    /// `None`, meaning no position information — tokenizers with no notion of morpheme
    /// position, like bigram, don't need to implement this (their matches are always
    /// literal substrings of the original text, so this feature isn't needed).
    fn locate(&self, _text: &str, _forms: &[String]) -> Option<(usize, usize)> {
        None
    }
}

/// Joins tokens into the form stored in the `content_fts.morph` column.
pub fn join_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| t.0.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
