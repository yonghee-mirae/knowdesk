# KnowDesk Architecture

# 목표

- Rust + Tauri 2 기반 인스톨러 배포 (v1.1 개정 — 단일 실행 파일 목표 폐기, `11_Implementation_Plan.md` 참조)
- 서버리스
- 로컬 전용
- 확장 가능한 DRM Adapter 구조

인스톨러 배포로 전환한 이유는 Kiwi(형태소 분석)와 PDFium(PDF 추출)이 네이티브 동적 라이브러리 + 모델/리소스 동봉을 요구하기 때문이다.

---

# Crate Structure

```text
knowdesk/
├── core/        # 순수 비즈니스 로직. Tauri를 절대 참조하지 않는다.
│   ├── config.rs
│   ├── db/      schema.rs migrate.rs documents.rs search_repo.rs
│   ├── scan/    walker.rs filter.rs hash.rs
│   ├── extract/ mod.rs(trait) txt.rs ooxml.rs xlsx.rs pdf.rs
│   ├── nlp/     mod.rs(trait) kiwi.rs bigram.rs synonym.rs
│   ├── index/   pipeline.rs queue.rs watcher.rs
│   └── search/  parser.rs service.rs rank.rs snippet.rs
├── cli/         # 헤드리스 검증 하니스 (index / search / stats / bench)
├── src-tauri/   # 트레이 상주, 전역 단축키, IPC
└── frontend/    # TS + Web Components + Vite
```

`core`는 Tauri API를 알아서는 안 된다. 모든 OS 통합(트레이, 전역 단축키, 파일 다이얼로그 등)은 `src-tauri`에 격리한다.

`cli`는 UI 없이 파이프라인 전체(`index` / `search` / `stats` / `bench`)를 구동하는 헤드리스 검증 도구다. Windows 환경 없이도 코어 로직을 자동 테스트할 수 있게 한다.

---

# Application Layers

Application

├── UI Layer

├── Search Layer

├── Index Layer

├── Extraction Layer

├── Repository Layer

└── Infrastructure Layer

---

# UI Layer

구성

- Search Window
- Preview Pane
- Settings Window
- Statistics Window
- Tray Manager

---

# Search Layer

구성

## SearchService

역할

- Query 해석
- FTS 실행
- 결과 랭킹

## QueryParser

지원

- Phrase
- AND
- OR
- NOT
- Prefix
- Filter

---

# Index Layer

## IndexService

역할

- 파일 수집
- 색인 생성
- 재색인

## Index Queue

비동기 처리

---

# Extraction Layer

## ContentExtractor (trait)

```rust
pub trait ContentExtractor {
    fn supports(&self, ext: &str) -> bool;
    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError>;
}
```

---

구현

### NonDrmExtractor

### DrmApiExtractor

### TrustedProcessExtractor

---

# NLP Layer

## Tokenizer (trait)

형태소 분석기를 교체 가능하게 추상화한다. MVP 초반은 `BigramTokenizer`로 시작하고, Kiwi 연동 검증 후 `KiwiTokenizer`로 교체한다.

```rust
pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}
```

구현

### BigramTokenizer (MVP 초기)

### KiwiTokenizer (Kiwi 연동, `Kiwi::from_config`로 오프라인 초기화)

---

# Repository Layer

## DocumentRepository

## SearchRepository

## DocumentStore (trait)

원문 저장 방식을 추상화한다. 초기에는 원문을 그대로 저장하고, 필요 시 압축 저장으로 전환할 수 있게 한다.

```rust
pub trait DocumentStore {
    fn put_body(&self, doc: DocId, text: &str) -> Result<()>;
    fn get_body(&self, doc: DocId) -> Result<Option<String>>;
}
```

---

# Infrastructure

## SQLite (rusqlite, bundled FTS5)

## notify (파일 감시 — FileSystemWatcher의 크로스플랫폼 대체)

## Kiwi (kiwi-rs, 오프라인 모델 동봉)

## pdfium-render (PDF 추출)

## Logging

---

# Sequence

File Added

↓

notify (File Watcher)

↓

Index Queue

↓

Extractor

↓

Tokenizer (Kiwi)

↓

FTS5

↓

Indexed
