# Coding Agent Backlog

v1.1 개정 — `06_Development_Roadmap.md`의 Phase A~D 순서에 맞춰 재배열했다. 기존 TASK ID는 최대한 유지하고, 새로 필요해진 항목만 각 블록 안에 번호를 추가했다.

---

# Phase A — Walking Skeleton

## Foundation

TASK-001 Create Cargo Workspace (core / cli / src-tauri / frontend)

TASK-002 Configure Logging

TASK-003 Create Configuration System

TASK-004 Create CLI Skeleton (index / search / stats / bench 서브커맨드)

---

## Storage

TASK-101 Create SQLite Schema (`documents` / `paths` 분리 반영)

TASK-102 Create Migration Runner

TASK-103 Create DocumentRepository

TASK-104 Create SearchRepository

TASK-105 Implement Index Pipeline & State Machine (FULL / META / SKIP)

---

## Discovery

TASK-201 Implement DirectoryScanner

TASK-202 Implement FileFilter

TASK-203 Implement SHA256Service

---

## Extraction (TXT만)

TASK-401 Create ContentExtractor trait

TASK-406 TXT Extractor + 인코딩 감지 (`encoding_rs` + `chardetng`, CP949/EUC-KR 대응)

---

## NLP (bigram)

TASK-501 Create Tokenizer trait

TASK-506 BigramTokenizer (MVP 초기 구현)

---

## Search

TASK-601 Query Parser

TASK-602 Search Service

TASK-603 Filename Search

TASK-604 Content Search (bm25 Ranking)

TASK-605 Snippet Generator

---

# Phase B — 실사용 가능한 코어

## Extraction 확장 (완료)

순서: XLSX → DOCX/PPTX → PDF

TASK-402 XLSX Extractor (`calamine`)

TASK-403 DOCX Extractor (`zip` + `quick-xml`)

TASK-404 PPTX Extractor (`zip` + `quick-xml`)

TASK-405 PDF Extractor (`pdfium-render`) — 한글 CID 폰트 별도 검증 필요 → 실제 CID 폰트 PDF로 검증 완료

---

## NLP 확장

TASK-502 Synonym Engine — 인터페이스(`SynonymDictionary` 트레이트)만 정의, 구현 보류(2026-08-21, 지금은 불필요하다는 판단). 사전 파일 형식·로딩·`search::service` 연결은 미착수

TASK-503 KiwiTokenizer (`Kiwi::from_config` 오프라인 초기화) — 완료. 네이티브 라이브러리는 v0.22.2 고정(`11_Implementation_Plan.md` 참조)

TASK-504 bigram 대비 Kiwi 재현율 비교 측정 — 질적 비교(테스트 케이스)로 완료. 대규모 정량 측정은 TASK-903(벤치마크)으로 이동

TASK-505 bigram(기본)+Kiwi(보조) 이원화 재설계 — `content_fts.morph`/`morph_kiwi` 컬럼 분리, 검색어 형태소 분석 확장, 매칭 근거("정확 일치"/"형태소 분석") 표시 — 완료

TASK-507 스니펫 원문 강조 보강 — 리터럴 검색어 → Kiwi 분석 어간 → 형태소 위치(`Tokenizer::locate`, 불규칙 활용형 대응) 순으로 원문에서 강조 위치를 찾음 — 완료

---

## Monitoring (완료)

TASK-301 Implement FileSystemWatcher (`notify`) — 완료 (`core/src/index/watcher.rs`)

TASK-302 Implement EventQueue — 완료 (`core/src/index/queue.rs`), 문서 삭제 시 orphan 정리 포함

TASK-303 Debounce (Office 저장 시 임시파일 폭풍 대응) — 완료. `notify-debouncer-mini`/`-full` 둘 다 무한 재색인 루프 문제가 있어(직접 확인) 직접 구현으로 변경. 상세 근거는 `06_Development_Roadmap.md` B4 참조

TASK-304 `cli watch` 서브커맨드 — 완료 (헤드리스 검증용)

TASK-305 경로 정규화 버그 수정 — 완료. 최초 스캔(사용자가 준 경로)과 `notify` 이벤트(cwd를 붙인 경로)의 문자열 표현이 달라 같은 파일이 문서 두 개로 나뉘어 색인되고, 내용을 수정해도 예전 내용이 검색에 영구히 남는 버그를 실사용 중 발견. `canonical_path`(`core/src/index/mod.rs`)로 수정, 상세 근거는 `06_Development_Roadmap.md` B4 참조

---

## Diagnostics 일부

TASK-903 Benchmark Harness (`cli bench` — 색인 처리량, 검색 P95, DB 실측 크기) — 완료. 대량 코퍼스는 `core/examples/gen_bench_corpus.rs`로 생성, 상세 근거는 `06_Development_Roadmap.md` B5 참조

---

# Phase C — UI

## UI

TASK-701 Search Window

TASK-702 Result List

TASK-703 Preview Pane (+ Highlight + Snippet)

TASK-705 File/Folder Actions (Open File / Open Folder / Copy Path — 키보드 전용)

TASK-704 Settings Window

---

## Tray

TASK-801 Tray Manager

TASK-802 Hotkey Manager (창 사전 생성 + show/focus 방식 — P95 300ms 대응)

---

## Diagnostics

TASK-901 Statistics Service

TASK-902 Log Export

---

# Phase D — Windows 이관

TASK-1001 Kiwi / PDFium Windows 바이너리 동봉 및 오프라인 초기화 경로

TASK-1002 Windows 경로 처리 (대소문자 정규화, 260자 초과, UNC 오프라인 처리)

TASK-1003 인스톨러 + 코드사이닝

TASK-1004 P95 성능 실측 및 튜닝

TASK-1005 DRM 적용률 실측 (O-4)
