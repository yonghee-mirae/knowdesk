# Development Roadmap

v1.1 개정 — M0~M15 선형 순서 대신, **동작하는 end-to-end 파이프라인을 최대한 빨리 세우는 순서**로 재배열했다 (`11_Implementation_Plan.md` 참조). 추출 포맷을 전부 만든 뒤 검색을 붙이면 마지막까지 아무것도 검증할 수 없기 때문이다. Windows 환경 없이 Linux에서 검증 가능한 항목을 Phase A~C에 최대한 몰아넣고, Windows 종속 항목만 Phase D로 분리했다.

---

# Phase A — Walking Skeleton (Linux 전량 검증 가능, 완료)

## S-1 PDF 한글 추출 스파이크

DRM을 논외로 하면 최대 리스크는 PDF 한글 추출 품질이다. 본구현(B1)이 아니라 사전 검증 스파이크(1일 내외)로, 본격 구현 전에 CID 폰트·다단 레이아웃·표·스캔본(이미지 PDF) 샘플에서 `pdfium-render`로 한글이 정상 추출되는지 먼저 확인한다. 스캔본은 OCR이 Out of Scope이라 텍스트가 비어 있을 수 있음을 미리 확인해두는 것도 포함(`04_Data_Model.md`의 `EMPTY_TEXT` 강등 사유 참조).

## S-2 Kiwi 메모리 실측 스파이크 (완료, 2026-08-23)

유휴 메모리 200MB 목표(PRD 4장)의 실현 가능성이 미검증이다. 본구현이 아니라 `Kiwi::from_config`로 모델을 로드한 뒤 RSS를 실측하는 스파이크(1일 내외)로, 목표치를 확정하기 전에 Kiwi 모델 자체의 메모리 사용량부터 파악한다.

⚠️ **실측 결과: 목표 미달성 확정.** `knowdesk-cli`를 빌드해 `/usr/bin/time -l`로 직접 측정 — Kiwi 로드 전 ~9.9MB, 로드 후 **~824MB RSS**(디스크상 모델 크기는 ~95MB에 불과, 8~9배 부풀어 오름). Apple Silicon(neon)에서 "Quantization is not supported for ArchType::neon. Fall back to non-quantized model." 경고와 함께 비양자화 모델로 폴백하는 게 원인으로 보인다. 게다가 `kiwi_rs::Kiwi`가 `!Send`라 `SearchWorker`와 색인/감시 워커가 각자 별도 인스턴스를 로드해 총 ~1.6GB까지 치솟는 문제도 함께 발견 — `KiwiActor`(전용 스레드 하나가 인스턴스를 소유하고 양쪽 워커가 채널로 요청만 보냄, `src-tauri/src/lib.rs`)로 공유해 인스턴스 하나로 줄였다. 그래도 여전히 200MB 목표를 훨씬 초과하므로, `enable_morphological_analysis` 설정(기본 off, `12_UI_Spec.md`)을 추가해 켜지 않으면 Kiwi를 아예 로드하지 않도록 했다 — PRD 4장 목표는 이 설정이 꺼진 기본 상태 기준으로 재해석한다.

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
- ⚠️ **정리 범위 확장 (2026-08-23):** 위 orphan 정리는 원래 "감시 중 파일 하나가 실제로 사라짐"만 다뤘는데, 실사용 중 그보다 넓은 케이스들이 안 잡히는 걸 발견해 전부 메웠다 — (1) 폴더째 삭제(파일 하나가 아니라 폴더 자체가 없어지면 그 파일의 부모까지 같이 사라져 `canonical_path`의 기존 1단계 복원이 실패하던 문제 — 조상을 계속 거슬러 올라가도록 일반화, `remove_paths_under`로 하위 전부 정리), (2) **앱이 꺼져 있는 동안** 파일/폴더가 삭제된 경우(라이브 `notify` 이벤트가 있을 수 없으므로, 앱 시작 시 감시 폴더마다 `prune_missing_paths_under`로 디스크 존재 여부를 재확인), (3) `watched_folders`에서 폴더를 뺀 경우 — 앱이 켜져 있든 꺼져 있든(`prune_paths_outside_watched`를 `apply_folder_diff` 호출마다 무조건 실행해 현재 설정 기준으로 전체 재정합), (4) 파일 **내용만** 바뀐 경우(`document_id`가 SHA256 해시라 내용이 바뀌면 새 문서로 취급되는데, 예전 `document_id`의 `documents`/`content_fts`/`document_bodies` 행이 고아로 영구히 남던 버그 — `upsert_path`가 재지정 직전의 이전 `document_id`를 기억해뒀다가 정리). 삭제로 비워진 공간이 `.db` 파일 크기에 실제로 반영되도록 `Db::reclaim_space()`(FTS5 `optimize` + `VACUUM` + WAL 체크포인트)도 추가 — `PRAGMA incremental_vacuum`은 이 환경에서 실측상 거의 동작하지 않아(N을 얼마로 줘도 호출당 페이지 1개 정도만 회수) 전체 `VACUUM`으로 전환했다.
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

⚠️ **macOS 선행 구현 (2026-08-23):** Windows 이관보다 먼저, macOS 패키징 요청에 맞춰 Kiwi/PDFium 동봉을 macOS용으로 구현했다 - `tauri.macos.conf.json`(macOS 전용 오버라이드, 아래 참조)의 `bundle.resources`가 `libpdfium.dylib`/`libkiwi.dylib`/Kiwi 모델을 `.app`의 `Contents/Resources/native/`에 동봉하고, `src-tauri`의 `set_bundled_native_lib_env_vars`(`run()` 맨 앞에서 1회 호출)가 실행 파일 경로 기준으로 그 위치를 계산해 `KNOWDESK_PDFIUM_LIB_DIR`/`KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`를 설정한다 - 이미 사용자가 직접 그 환경변수를 설정해 둔 경우엔 손대지 않고, 동봉 파일이 실제로 없으면(dev 빌드) 조용히 아무것도 하지 않는다(둘 다 이미 있던 graceful fallback: PDF는 META, Kiwi는 bigram만). 패키지된 `.app`으로 실제 색인해 `morph_kiwi` 컬럼에 형태소 분석 결과("지었다"→"짓")가 들어가는 것, PDF가 META가 아니라 FULL로 색인되는 것 모두 확인함.

⚠️ **플랫폼별 설정 분리 + Windows 리소스 동봉 추가 (2026-08-24):** 공용 `tauri.conf.json`에 macOS `.dylib` 경로가 하드코딩돼 있던 걸 Tauri 2의 플랫폼별 설정 오버라이드(`tauri.<platform>.conf.json`, 해당 플랫폼 빌드에만 병합됨)로 분리 - `bundle.resources`/`bundle.targets`를 `tauri.macos.conf.json`으로 옮기고, 같은 패턴으로 `tauri.windows.conf.json`을 신설해 `pdfium.dll`/`kiwi.dll`/Kiwi 모델 동봉을 추가했다. `src-tauri`에도 Windows용 `set_bundled_native_lib_env_vars`를 macOS 버전과 나란히 추가 - Tauri 자체의 `resource_dir()` 규칙상 Windows는 macOS(`Contents/Resources`)·Linux(`/usr/lib/<name>`)와 달리 **실행 파일과 같은 폴더**가 리소스 위치라, 상대경로 계산이 그만큼 더 단순하다. ⚠️ **미검증** - 이 환경에는 Windows 머신이 없어 실제로 돌려보지 못했다. 파일 배치는 `env.ps1`에 이미 있던 가정을 그대로 따른다: `pdfium.dll`은 `bin/` 폴더 아래(pdfium-binaries Windows 배포판의 실제 폴더명 미확인 - mac/Linux는 `lib/`로 확인됨), `kiwi.dll`은 `lib/` 바로 아래(`scripts/install_kiwi.ps1`로 확인됨, `libkiwi.{so,dylib}`와 달리 `lib` 접두사 없음). 실제 Windows 빌드로 검증 전까지는 이 경로 가정이 맞는지 알 수 없다.

## D2 Windows 경로 처리

- 경로 대소문자 정규화
- 260자 초과 경로 (`\\?\` 접두)
- UNC 네트워크 드라이브 오프라인 처리

## D3 Packaging

- 인스톨러
- 코드사이닝

⚠️ **macOS 선행 구현 (2026-08-23):** `tauri.conf.json`의 `bundle.active`를 켜고 `targets: ["app", "dmg"]`(2026-08-24부터 `tauri.macos.conf.json`으로 이동, 위 D1 참조)로 `.app`+`.dmg`를 만든다(코드사이닝은 아직 없음 - 로컬 실행/배포 테스트용 ad-hoc 빌드).

⚠️ **Windows `bundle.targets` 추가 (2026-08-24, 미검증):** `tauri.windows.conf.json`에 `targets: ["msi", "nsis"]` 추가 - Tauri 2가 지원하는 두 Windows 인스톨러 포맷 모두를 대상으로 한다. 실제로 빌드해본 적은 없음(Windows 머신 없음) - `cargo build`/`cargo clippy`가 이 Linux 머신에서 통과하는 것만 확인했고, 인스톨러 산출물 자체·코드사이닝은 여전히 미착수.

## D4 Performance

- P95 성능 실측 및 튜닝

## D5 DRM 실측

- DRM 적용률 실측 (O-4) → Phase 2 선행 여부 판단
