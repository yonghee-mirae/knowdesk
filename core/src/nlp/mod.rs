//! `Tokenizer` — 형태소 분석기 추상화 (`docs/08_API_Contracts.md`).
//! MVP 초기는 `BigramTokenizer`로 시작하고, Phase B에서 `KiwiTokenizer`로 교체한다.

pub mod bigram;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub String);

pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}

/// `content_fts.morph` 컬럼에 저장할 형태로 토큰을 합친다.
pub fn join_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| t.0.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
