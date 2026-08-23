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

`core/src/index/queue.rs`(Phase B4). `notify` 이벤트를 디바운스한 경로 묶음을 받아 파일별로 `IndexPipeline::index_file`(존재하면) 또는 삭제 처리(사라졌으면)로 분기한다. 문서 삭제 시 그 문서를 참조하는 다른 경로가 더 없으면 `documents`/`content_fts`/`document_bodies`까지 정리(orphan GC) — 단, 네트워크 드라이브 대량 오프라인과 실제 삭제를 구분하는 문제(D-1, 미결)는 다루지 않는다.

`paths` 테이블은 경로 문자열이 기본 키인데, 최초 전체 스캔(사용자가 준 경로 그대로)과 `notify` 이벤트(cwd를 붙인 경로)가 같은 파일을 다른 문자열로 표현할 수 있다 — 실사용 중 이 때문에 같은 파일이 문서 두 개로 나뉘어 색인되고, 내용을 수정해도 예전 내용이 검색에 영구히 남는 버그가 실제로 있었다. `IndexPipeline::index_file`과 `queue`가 경로를 항상 `canonical_path`(`core/src/index/mod.rs`)로 정규화하도록 고쳤다 — 파일이 있으면 그대로 canonicalize, 삭제돼서 canonicalize가 안 되면 부모 디렉터리(그마저 없으면 더 위 조상까지)를 canonicalize해서 재구성한다.

⚠️ **정리 범위 확장 (2026-08-23):** 위의 "단일 경로 삭제" 외에, 실사용 중 인덱스 DB가 정리되지 않는 경우가 더 있다는 게 확인돼 아래 세 경로를 추가했다(상세 근거·테스트는 `06_Development_Roadmap.md` B4 항목, 구현은 `DocumentRepository::remove_paths_under`/`prune_missing_paths_under`/`prune_paths_outside_watched`, `core/src/db/documents.rs`).

- **폴더째 삭제**: `notify`가 개별 파일 삭제 이벤트를 다 안 보내주는 경우가 있어, `queue::handle_path`가 사라진 경로에 대해 정확히 일치하는 경로 삭제뿐 아니라 그 경로를 디렉터리 접두사로 갖는 하위 경로까지 함께 정리한다.
- **앱이 꺼져 있는 동안의 변경**: 앱 시작 시 각 감시 폴더에 대해 DB에는 있지만 실제 디스크에는 없는 경로를 한 번 훑어 정리한다(`prune_missing_paths_under`). 내용이 수정된 파일(= 새 `document_id`로 재색인)도 재색인 시점에 이전 `document_id`가 더 이상 어떤 경로에서도 참조되지 않으면 함께 정리한다(`upsert_path`가 갱신 전 `document_id`를 읽어 두었다가 갱신 후 orphan 여부를 확인).
- **감시 폴더가 설정에서 제외됨**: 앱이 켜져 있든 꺼져 있든, 더 이상 어떤 감시 폴더에도 속하지 않는 인덱스 경로를 전부 정리한다(`prune_paths_outside_watched`) — 예전에는 "폴더 제외는 의도적으로 정리 대상에서 뺀다"는 주석이 있었으나, 이는 사용자가 요구한 적 없는 임의의 설계였다고 판단해 폐기했다.

이 세 경로 모두 문서를 제거한 뒤에는 `Db::reclaim_space()`(FTS5 optimize + `VACUUM` + `PRAGMA wal_checkpoint(TRUNCATE)`)를 호출해 실제 DB 파일 크기도 줄어들도록 한다 — 상세는 `04_Data_Model.md`/`11_Implementation_Plan.md` 참조.

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

⚠️ **`enable_morphological_analysis` 설정 추가 (2026-08-23):** `KiwiTokenizer`(정확히는 내부 `kiwi-rs` `Kiwi`)를 로드하면 인스턴스당 RSS가 실측 ~824MB까지 치솟는 문제가 확인됐다(Apple Silicon에서 양자화 모델을 못 써 비양자화 모델로 폴백하는 게 원인 — 상세는 `06_Development_Roadmap.md` S-2, `07_Coding_Agent_Backlog.md` TASK-006 참조). 이를 완화하기 위해 `settings.json`에 `enable_morphological_analysis`(기본값 `false`) 설정을 추가했고, 꺼져 있으면 `KiwiTokenizer`를 아예 로드하지 않는다. `Kiwi`는 `!Send`라 여러 스레드(색인/검색 워커)가 공유할 수 없는데, `src-tauri`의 `KiwiActor`(전용 스레드에서 유일한 `Kiwi` 인스턴스를 소유)와 `KiwiHandle`(`Clone` 가능한 `mpsc::Sender` 래퍼, `Tokenizer` 구현)로 이 문제를 해결해 인스턴스를 하나만 유지한다.

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
