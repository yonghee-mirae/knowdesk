# Development Roadmap

v1.1 개정 — M0~M15 선형 순서 대신, **동작하는 end-to-end 파이프라인을 최대한 빨리 세우는 순서**로 재배열했다 (`11_Implementation_Plan.md` 참조). 추출 포맷을 전부 만든 뒤 검색을 붙이면 마지막까지 아무것도 검증할 수 없기 때문이다. Windows 환경 없이 Linux에서 검증 가능한 항목을 Phase A~C에 최대한 몰아넣고, Windows 종속 항목만 Phase D로 분리했다.

---

# Phase A — Walking Skeleton (Linux 전량 검증 가능, 완료)

## S-1 PDF 한글 추출 스파이크

DRM을 논외로 하면 최대 리스크는 PDF 한글 추출 품질이다. 본구현(B1)이 아니라 사전 검증 스파이크(1일 내외)로, 본격 구현 전에 CID 폰트·다단 레이아웃·표·스캔본(이미지 PDF) 샘플에서 `pdfium-render`로 한글이 정상 추출되는지 먼저 확인한다. 스캔본은 OCR이 Out of Scope이라 텍스트가 비어 있을 수 있음을 미리 확인해두는 것도 포함(`04_Data_Model.md`의 `EMPTY_TEXT` 강등 사유 참조).

## S-2 Kiwi 메모리 실측 스파이크

유휴 메모리 200MB 목표(PRD 4장)의 실현 가능성이 미검증이다. 본구현이 아니라 `Kiwi::from_config`로 모델을 로드한 뒤 RSS를 실측하는 스파이크(1일 내외)로, 목표치를 확정하기 전에 Kiwi 모델 자체의 메모리 사용량부터 파악한다.

## A1 Foundation

- Cargo Workspace 구성 (core / cli / src-tauri / frontend)
- Logging (로그 마스킹 포함 — 문서 본문·검색어는 로그에 기록하지 않는다, `KnowDesk_추가검토사항.md` E-3 참조)
- Configuration
- CLI Skeleton (index / search / stats / bench)

## A2 Storage

- SQLite Schema (`documents` / `paths` 분리, `04_Data_Model.md` 참조)
- Migration Runner
- Repository
- Index Pipeline & State Machine (FULL / META / SKIP)

## A3 Discovery

- Folder Scan
- Filters
- SHA256

## A4 Extraction (TXT만)

- ContentExtractor trait
- TXT Extractor + 인코딩 감지 (CP949/EUC-KR/UTF-8)

## A5 NLP (bigram)

- Tokenizer trait
- BigramTokenizer (MVP 초기 구현)

## A6 Index Pipeline 통합

- IndexService 구현체 연결

## A7 Search

- Query Parser
- FTS5 Search
- bm25 Ranking
- Snippet Generator

**완료 기준:** `cli index ./samples && cli search "채권 발행"`이 스니펫과 함께 결과를 반환한다.

---

# Phase B — 실사용 가능한 코어

## B1 Extraction 확장 (완료)

순서: XLSX → DOCX/PPTX → PDF. 구현 순서 자체는 그대로 두되(PDF가 가장 복잡한 포맷), 최대 리스크인 한글 CID 폰트 문제는 본구현 전에 S-1 스파이크로 먼저 검증한다 — "리스크가 크니 마지막에 검증"이 아니라 "리스크가 크니 먼저 검증"으로 순서를 바꿨다.

- XLSX (calamine)
- DOCX / PPTX (zip + quick-xml)
- PDF (pdfium-render) — 실제 한글 CID 폰트 PDF로 검증 완료

## B2 Kiwi 연동 (완료)

- KiwiTokenizer (`Kiwi::from_config`, 오프라인 초기화 — 망분리 대응)
  - ⚠️ Kiwi 네이티브 라이브러리는 **v0.22.2** 고정 (`v0.23.2`는 `kiwi-rs`와 ABI가 안 맞아 세그폴트 — `11_Implementation_Plan.md` 참조)
- bigram(항상 실행하는 기본 토크나이저)과 Kiwi(가능할 때만 실행하는 보조 토크나이저)를 **택일이 아니라 병행**하는 것으로 재설계 (`content_fts.morph`/`morph_kiwi` 컬럼 분리)
- 검색어도 Kiwi로 형태소 분석해 확장 (`(원문 OR morph_kiwi:(분석 형태소...))`) — bigram은 검색어 분석에 안 씀
- 히트별로 "정확 일치"/"형태소 분석" 매칭 근거 표시
- 스니펫 강조: 리터럴 → Kiwi 분석 어간 → `Tokenizer::locate`(형태소 위치, 불규칙 활용형 대응) 순으로 원문에서 강조 위치를 찾음
- bigram 대비 재현율 비교 — 질적 사례(테스트)로 확인 완료. 대규모 정량 측정은 B5 벤치마크로 이동

## B3 동의어 사전 (인터페이스만, 구현 보류)

- Synonym Engine (질의 시점 확장)
- 2026-08-21: 지금은 이 기능이 필요하지 않다는 판단에 따라 `SynonymDictionary` 트레이트(`core/src/nlp/synonym.rs`)만 정의하고 실제 구현(사전 파일 형식, 로딩, `search::service` 연결)은 보류. `KnowDesk_추가검토사항.md` D-3(사용자 등록 기능 제공 여부)도 여전히 미결이라, 구현 시점에 그것부터 정해야 한다.

## B4 File Monitoring (완료)

- FileSystemWatcher (notify)
- EventQueue
- Debounce (Office 저장 시 임시파일 폭풍 대응 — 필수)
- ⚠️ 디바운스는 `notify-debouncer-mini`/`-full` 둘 다 안 쓰고 **직접 구현**했다 — 처음엔 `notify-debouncer-mini`로 구현했다가 **무한 재색인 루프**를 실제로 재현했다. Linux inotify 백엔드는 `OPEN`/`ATTRIB`(접근·메타데이터 변경)까지 기본으로 감시하는데, 색인 파이프라인이 파일을 읽는 것(해시 계산, 텍스트 추출) 자체가 `OPEN` 이벤트를 만들어 "읽음→이벤트 발생→재색인→다시 읽음"이 끝없이 돈다. `notify-debouncer-full`도 소스를 확인해봤는데 `EventKind::Access`/`Modify(Metadata)`를 걸러내지 않아 동일한 문제가 있다. 그래서 원시 `notify::Event`를 직접 받아 `EventKind`로 필터링(Create/Remove/Modify(Data\|Name)만 통과)한 뒤 직접 디바운스한다 (`core/src/index/watcher.rs`). 문서 식별이 경로가 아니라 내용 해시 기준이라 rename 전용 추적(`-full`이 제공하는 기능)도 필요 없다.
- ⚠️ **경로 정규화 버그 발견·수정 (2026-08-21):** `watch` 사용 중 사용자가 실제로 겪은 버그 — 파일 내용을 수정해도 예전 내용이 검색에 영구히 남았다. 원인: 최초 전체 스캔(`run_index`)은 사용자가 준 경로 문자열(예: `./samples/x.txt`)을 그대로 쓰지만, `notify`가 그 뒤 변경을 알릴 땐 현재 작업 디렉터리를 붙인 경로(`/현재/디렉터리/./samples/x.txt`)로 이벤트를 준다. `paths` 테이블은 경로 문자열이 기본 키라서 같은 파일이 문서 두 개로 나뉘어 색인되고, 내용이 바뀌어도 예전 문서가 정리되지 않은 채 검색에 계속 노출됐다. `IndexPipeline::index_file`/`queue`에서 경로를 항상 `canonicalize`하도록 고쳤다(`core/src/index/mod.rs`의 `canonical_path`) — 삭제된 파일은 canonicalize가 안 되므로 부모 디렉터리만 canonicalize해서 재구성한다.
- 문서 삭제 시 orphan 정리(`DocumentRepository::remove_path`) — 다른 경로가 그 문서를 더 안 참조하면 `documents`/`content_fts`/`document_bodies`까지 정리. 네트워크 드라이브 대량 오프라인과 실제 삭제를 구분하는 문제(D-1, 미결)는 범위 밖 — 지금은 경로 하나가 사라지면 그대로 삭제로 처리한다.
- 헤드리스 검증용 `cli watch <경로>` 서브커맨드 추가.
- 색인 스로틀링 — 워커 수 제한 + 배치 간 sleep으로 초기 대량 색인이 유휴 CPU 목표(PRD 4장, 1% 미만)를 침해하지 않게 한다.

## B5 Benchmark (완료)

- Benchmark Harness (`cli bench <경로> [--queries 파일] [--repeat N]`) — 색인 처리량, 검색 P95(기준: PRD 4장 "검색 응답 P95 1초 이내" 대비 PASS/FAIL 표시), DB 실측 크기(원본 대비 배율)
- "검색창 호출 P95 300ms"·유휴 CPU/메모리는 트레이·전역 단축키·상주 프로세스가 있어야 실측 가능해서 범위 밖(Phase C/D)
- 검색 벤치마크용 검색어는 `--queries` 파일(한 줄에 하나)로 주거나, 생략하면 내부 기본 세트(키워드/구문/AND/OR/NOT/접두 각 1개) 사용
- 대량 코퍼스가 필요해 `core/examples/gen_bench_corpus.rs` 신설 — `gen_samples`(포맷 커버리지용, 파일 10여 개)와 별도로, 개수·총 용량 규모가 목적이라 `.txt`만 대량(기본 5,000건) 생성. 포맷별 추출 정확성은 `gen_samples`/익스트랙터 테스트가 이미 커버
- `--db`가 비어있어야 처리량 숫자가 의미 있음(이미 색인된 db에 돌리면 대부분 SKIP) — 안내 문구로 남김

---

# Phase C — UI (Linux에서 대부분 검증 가능)

## C1 Search UI

- Search Window
- Result List

## C2 Preview

- Preview Pane
- Highlight
- Snippet

## C3 Actions

- Open File / Open Folder / Copy Path — 전부 키보드만으로

## C4 Tray & Hotkey

- Tray Integration
- Global Shortcut (창을 미리 생성해 숨겨두고 show+focus만 수행 — P95 300ms 대응)

## C5 Settings & Diagnostics

- Index Worker (완료, TASK-706) — `src-tauri`가 `Config.watched_folders`를 읽어 시작 시 전체 스캔 + 이후 계속 감시.
- Settings Window (완료, TASK-704 — Settings Window 대신 "설정 파일 폴더 열기"로 대체) — 폴더 추가/제거 UI는 만들었다가 걷어냄. 지금은 트레이/검색바의 "설정"이 `settings.json`이 있는 폴더를 OS 파일 관리자로 열어주고, 사용자가 그 파일을 텍스트 에디터로 직접 편집한다. 앱 시작 시 파일이 없으면 기본값으로 생성.
- Statistics (완료, TASK-901) — 트레이 메뉴 "Statistics" 액션, `07_Coding_Agent_Backlog.md` 참조.
- ~~Logs~~ **TASK-902 폐기 (2026-08-24)** - 불필요하다고 판단, 구현된 적 없음.
- 초기 색인 진행률 표시 (완료, TASK-904) — 온보딩 위저드는 아니고, 최초 대량 색인 중 진행률과 "색인 중" 상태 문구만 검색창 상단 배너로 노출한다(`KnowDesk_추가검토사항.md` E-2, `07_Coding_Agent_Backlog.md` 참조)

---

# Phase D — Windows 이관 (여기서 처음 Windows 필요)

## D1 Native Dependency 배포

- Kiwi / PDFium Windows 바이너리 동봉
- 오프라인 초기화 경로 검증

## D2 Windows 경로 처리

- 경로 대소문자 정규화
- 260자 초과 경로 (`\\?\` 접두)
- UNC 네트워크 드라이브 오프라인 처리

## D3 Packaging

- 인스톨러
- 코드사이닝

## D4 Performance

- P95 성능 실측 및 튜닝

## D5 DRM 실측

- DRM 적용률 실측 (O-4) → Phase 2 선행 여부 판단
