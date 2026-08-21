//! `Tokenizer` — 형태소 분석기 추상화 (`docs/08_API_Contracts.md`).
//! MVP 초기는 `BigramTokenizer`로 시작하고, Phase B에서 `KiwiTokenizer`로 교체한다.

pub mod bigram;
pub mod kiwi;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub String);

pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;

    /// `text` 안에서 `forms` 중 하나와 형태가 일치하는 형태소를 찾아, 그 형태소가
    /// 속한 어절 전체의 글자 위치(시작, 길이 — 원문 기준, 글자 단위)를 돌려준다.
    /// 활용형 때문에 표면형이 분석 결과와 달라도(예: "지었다"→어간 "짓") 원문에서
    /// 정확한 구간을 강조하기 위한 것이다 (`search::service`). 기본 구현은 위치
    /// 정보가 없다는 뜻으로 `None` — bigram처럼 형태소 위치 개념이 없는 토크나이저는
    /// 굳이 구현하지 않아도 된다(그런 토크나이저의 매칭은 항상 원문 그대로의
    /// 부분 문자열이라 이 기능이 필요 없다).
    fn locate(&self, _text: &str, _forms: &[String]) -> Option<(usize, usize)> {
        None
    }
}

/// `content_fts.morph` 컬럼에 저장할 형태로 토큰을 합친다.
pub fn join_tokens(tokens: &[Token]) -> String {
    tokens
        .iter()
        .map(|t| t.0.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}
