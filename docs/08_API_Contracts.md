# API Contracts

v1.1 개정 — C# 인터페이스 표기를 Rust trait으로 교체했다 (`11_Implementation_Plan.md` 참조). 시그니처는 확정이 아니라 설계 의도를 보이기 위한 스케치이며, 구현 시 세부 타입은 조정될 수 있다.

기존 `09_API_Contracts.md`는 본 파일과 내용이 완전히 동일한 중복 파일이었으므로 제거했다.

---

## ContentExtractor

본문 추출 추상화. `NonDrmExtractor` / `DrmApiExtractor` / `TrustedProcessExtractor`가 구현한다.

```rust
pub trait ContentExtractor {
    fn supports(&self, ext: &str) -> bool;
    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError>;
}
```

## Tokenizer

형태소 분석기 추상화. `BigramTokenizer`(기본, 항상 실행)와 `KiwiTokenizer`(보조, 가능할 때만 실행)를 함께 쓴다 — 택일이 아니다 (`11_Implementation_Plan.md` Phase B2 참조).

```rust
pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}
```

## SynonymDictionary

동의어 사전 추상화 (Phase B3). Kiwi가 못 잇는, 뜻은 같지만 글자가 다른 단어(사내 약어·전문용어 등)를 검색어 확장 시점에 연결한다. 인터페이스만 정의돼 있고 구현은 보류 상태다(`06_Development_Roadmap.md` B3, 2026-08-21 — 지금은 불필요하다는 판단).

```rust
pub trait SynonymDictionary {
    fn synonyms(&self, term: &str) -> Vec<String>;
}
```

## DocumentStore

원문 저장 방식 추상화. 원문 저장(구성 A) → 압축 하이브리드(구성 D) 전환을 재색인 없이 가능하게 한다.

```rust
pub trait DocumentStore {
    fn put_body(&self, doc: DocId, text: &str) -> Result<()>;
    fn get_body(&self, doc: DocId) -> Result<Option<String>>;
}
```

## IndexService

```rust
pub trait IndexService {
    fn index_document(&self, path: &Path) -> Result<(), IndexError>;
}
```

## SearchService

```rust
pub trait SearchService {
    fn search(&self, request: SearchRequest) -> Result<SearchResult, SearchError>;
}
```
