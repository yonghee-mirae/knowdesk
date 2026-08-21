//! `SynonymDictionary` — 동의어 사전 추상화 (Phase B3, `docs/06_Development_Roadmap.md`).
//!
//! Kiwi가 잇는 건 같은 단어의 활용형뿐이다(예: "짓다"→"지었다", `KiwiTokenizer`
//! 참조). 뜻은 같지만 글자가 완전히 다른 단어(사내 약어·전문용어 등, 예:
//! "ELS"↔"주가연계증권")는 형태소 분석으로 이을 수 없고, 명시적으로 등록해둔
//! 사전으로만 연결할 수 있다.
//!
//! 지금은 이 기능 자체가 필요하지 않다는 판단(2026-08-21)에 따라 인터페이스만
//! 정의해두고 구현은 보류한다 — `KnowDesk_추가검토사항.md` D-3("사내 약어 동의어
//! 사전 — 사용자 등록 기능 제공 여부")도 미결로 남아 있어, 파일 기반 읽기 전용
//! 사전인지 사용자 편집 UI까지 필요한지 등 구체적인 형태를 아직 정하지 않았다.
//! 나중에 구현하게 되면 `Tokenizer`와 같은 자리에서, `search::service`의 검색어
//! 확장 로직에 `Option<&dyn SynonymDictionary>`로 꽂으면 된다 — bigram(기본)과
//! Kiwi(보조)를 병행하는 지금 구조에 그대로 세 번째 확장 축으로 추가된다.

pub trait SynonymDictionary {
    /// `term`의 동의어 목록을 돌려준다. 없으면 빈 벡터.
    fn synonyms(&self, term: &str) -> Vec<String>;
}
