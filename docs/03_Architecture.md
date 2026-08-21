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

`core/src/index/queue.rs`(Phase B4). `notify` 이벤트를 디바운스한 경로 묶음을 받아 파일별로 `IndexPipeline::index_file`(존재하면) 또는 `DocumentRepository::remove_path`(사라졌으면)로 분기한다. 문서 삭제 시 그 문서를 참조하는 다른 경로가 더 없으면 `documents`/`content_fts`/`document_bodies`까지 정리(orphan GC) — 단, 네트워크 드라이브 대량 오프라인과 실제 삭제를 구분하는 문제(D-1, 미결)는 다루지 않는다.

`paths` 테이블은 경로 문자열이 기본 키인데, 최초 전체 스캔(사용자가 준 경로 그대로)과 `notify` 이벤트(cwd를 붙인 경로)가 같은 파일을 다른 문자열로 표현할 수 있다 — 실사용 중 이 때문에 같은 파일이 문서 두 개로 나뉘어 색인되고, 내용을 수정해도 예전 내용이 검색에 영구히 남는 버그가 실제로 있었다. `IndexPipeline::index_file`과 `queue`가 경로를 항상 `canonical_path`(`core/src/index/mod.rs`)로 정규화하도록 고쳤다 — 파일이 있으면 그대로 canonicalize, 삭제돼서 canonicalize가 안 되면 부모 디렉터리만 canonicalize해서 재구성한다.

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

형태소 분석기를 교체 가능하게 추상화한다. `BigramTokenizer`와 `KiwiTokenizer`는 "택일"이 아니라 역할이 다르다 — `BigramTokenizer`는 항상 실행되는 기본 토크나이저(`content_fts.morph`), `KiwiTokenizer`는 가능할 때만 추가로 붙는 보조 토크나이저(`content_fts.morph_kiwi`)다 (v1.1 대비 변경, Phase B2 — 근거는 `11_Implementation_Plan.md` 참조).

```rust
pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}
```

구현

### BigramTokenizer (기본, 항상 실행)

### KiwiTokenizer (보조, `Kiwi::from_config`로 오프라인 초기화, 가능할 때만 실행)

검색어 분석에는 `KiwiTokenizer`만 쓴다(가능할 때). bigram은 색인에서만 쓰는 기본 토크나이저이며, 검색어를 bigram으로 분석해도 "정확한 문구"·"비슷한 의미" 어느 쪽에도 도움이 되지 않고 짧은 음절 조각 때문에 정밀도만 떨어뜨린다 — 자세한 근거는 `11_Implementation_Plan.md` 참조.

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

`core/src/index/watcher.rs`(Phase B4). 디바운스는 `notify-debouncer-mini`/`-full` 둘 다 안 쓰고 직접 구현했다 — 둘 다 원시 이벤트를 걸러줄 지점을 열어주지 않아서, 색인 파이프라인이 파일을 읽는 것(해시 계산, 텍스트 추출) 자체가 만드는 `OPEN`/`ATTRIB` 이벤트까지 그대로 디바운스에 들어가 **무한 재색인 루프**가 생긴다(실제로 재현·확인함, `notify-debouncer-full`의 `add_event`도 `EventKind::Other`만 걸러내고 `Access`/`Modify(Metadata)`는 catch-all로 통과시킨다). 그래서 원시 `notify::Event`를 직접 받아 `EventKind`로 필터링(`Create`/`Remove`/`Modify(Data|Name)`만 통과)한 뒤 "마지막 이벤트 후 조용해지면 확정"하는 단순한 디바운스를 직접 구현한다. rename 전용 추적은 필요 없다 — 문서 식별이 경로가 아니라 내용 해시(SHA256) 기준이라, rename도 "옛 경로 제거 + 새 경로 추가"로 자연스럽게 처리된다.

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
