# Coding Agent Backlog

v1.1 개정 — `06_Development_Roadmap.md`의 Phase A~D 순서에 맞춰 재배열했다. 기존 TASK ID는 최대한 유지하고, 새로 필요해진 항목만 각 블록 안에 번호를 추가했다. Phase E는 이 재배열 이후 단독 배포 도구(`kdfind`)가 추가되며 새로 생겼다 — `06_Development_Roadmap.md`에도 동일하게 반영.

---

# Phase A — Walking Skeleton

## Spike (본구현 아님, 사전 검증 — `06_Development_Roadmap.md` S-1/S-2 참조)

TASK-005 PDF 한글 추출 스파이크 — `pdfium-render`로 CID 폰트·다단·표·스캔본 샘플 검증 (1일 내외)

TASK-006 Kiwi 메모리 실측 스파이크 — `from_config` 로드 후 RSS 측정 (1일 내외) — 완료 (2026-08-23). 인스턴스당 ~824MB RSS, 200MB 목표 미달성 확정. 상세 결과·대응은 `06_Development_Roadmap.md` S-2 참조

---

## Foundation

TASK-001 Create Cargo Workspace (core / cli / src-tauri / frontend)

TASK-002 Configure Logging

TASK-007 로그 마스킹 — 문서 본문·검색어가 로그에 기록되지 않도록 처리 (`KnowDesk_추가검토사항.md` E-3 참조)

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

TASK-306 색인 스로틀링 — 워커 수 제한 + 배치 간 sleep. 초기 대량 색인이 유휴 CPU 목표(PRD 4장)를 침해하지 않게 함 (`KnowDesk_추가검토사항.md` Part F 참조)

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

TASK-706 Index Worker (완료) — `src-tauri`에 배선된 색인/감시 백그라운드 워커. `Config.watched_folders` 목록을 앱 시작 시 전체 스캔 후 계속 감시(`core/src/index/watcher.rs`의 `FileWatcher::new`가 폴더 여러 개를 한 watcher/스레드에 묶어, 폴더 수만큼 `KiwiTokenizer` 인스턴스가 늘어나는 것을 막음). 폴더 목록을 채우는 UI(TASK-704)가 아직 없어서, 지금은 `KNOWDESK_SETTINGS_PATH`(또는 기본 위치의 `settings.json` — 색인 DB와 같은 폴더, 2026-08-22 결정)를 직접 편집해야 함 — 그 편집·삭제 자체는 아래 참조대로 자동 반영되므로 편집 후 별도 조작은 필요 없다. 목록이 비어 있으면(기본값) 워커는 항상 뜨지만 폴더가 없어서 아무것도 안 할 뿐이다.

⚠️ **재설계 이력:**
1. (2026-08-22) 트레이의 Reload가 앱 재시작(`AppHandle::restart()`) 방식이었을 때 `npm run tauri dev`에서 실제로 멈추는 문제가 발견되어(`12_UI_Spec.md` C4 참조), IndexWorker를 컨트롤 채널이 있는 상시 액터로 다시 만들었다(`SearchWorker`와 같은 모양) — `IndexCommand::Reload`를 받으면 스레드 안에서 `settings.json`을 다시 읽고 `apply_folder_diff`로 `watched_folders` 변경분만 적용.
2. (2026-08-23) 그 컨트롤 채널·트레이의 "Reload" 항목을 전부 없앴다 — "파일 감시 기능이 이미 있으니 설정 파일에도 그대로 적용해라"는 지시에 따라, `run_index_worker`가 색인 대상 폴더용 `FileWatcher`와 별개로 `settings.json`이 있는 폴더도 감시하도록 바꿈. 수정 이벤트가 오면 `reload_settings`로 다시 읽어 `apply_folder_diff` 적용(추가된 폴더는 스캔+`FileWatcher::watch`, 빠진 폴더는 `FileWatcher::unwatch`), 삭제 이벤트가 오면(파일이 실제로 없어졌는지 확인 후) `Config::default()`로 재생성 — 수동 "Reload" 자체가 필요 없어짐. `KiwiTokenizer`는 여전히 폴더가 실제로 생기기 전까지 로드를 미룬다(메모리 낭비 방지, `06_Development_Roadmap.md` S-2).
3. (2026-08-23) `KiwiTokenizer`는 폴더 수만큼 인스턴스가 늘어나는 건 막아뒀지만, **`SearchWorker`와 이 색인 워커가 서로 각자 인스턴스를 하나씩 로드하는 중복**은 그대로였다(둘 다 별도 스레드라 `!Send`인 `kiwi_rs::Kiwi`를 공유 못 함) — S-2 실측(인스턴스당 ~824MB)으로 총 ~1.6GB까지 치솟는 게 드러나 `KiwiActor`(전용 스레드 하나가 유일한 인스턴스를 소유, 양쪽 워커는 채널로 tokenize/locate 요청만 보냄)로 통합했다. 추가로 `enable_morphological_analysis` 설정(기본 off, `12_UI_Spec.md`)이 꺼져 있으면 이 액터조차 Kiwi를 로드하지 않는다.

TASK-704 Settings Window → "설정 파일 폴더 열기"로 대체 (완료, 2026-08-22) — Settings Window를 실제로 만들었다가(폴더 추가/제거 UI, `tauri-plugin-dialog` 네이티브 다이얼로그) 사용자 지시로 걷어내고, 훨씬 단순한 방식으로 교체했다: 트레이 메뉴/검색바 톱니바퀴의 "설정"이 이제 `settings.json`이 들어있는 폴더를 OS 파일 관리자로 열어주기만 하고(`open_settings_folder`), 그 파일을 텍스트 에디터로 직접 편집하는 게 UI 전체다. `run()`이 시작 시 `settings.json`이 없으면 `Config::default().save(...)`로 기본값 파일을 만들어둔다(빈 파일 상태로 시작하지 않게). 폴더 추가/제거 IPC 커맨드, 네이티브 폴더 선택 다이얼로그, 새 "settings" 창은 전부 제거됨.

⚠️ **버그 발견·제거(2026-08-22):** 되돌리기 전 버전에서 "폴더 추가" 클릭 시 앱 전체가 멈추는 문제가 실제로 발생했다. 원인: `tauri_plugin_dialog`의 `blocking_pick_folder()`를 **동기(non-async)** `#[tauri::command]` 안에서 호출함 — 동기 커맨드의 본문은 IPC 콜백이 온 스레드(WKWebView 기준 메인 스레드)에서 그대로 실행되는데, `blocking_pick_folder()`는 실제 다이얼로그 표시를 `run_on_main_thread`로 메인 스레드에 넘기고 그 결과를 **동기적으로 대기**한다 — 호출자가 이미 메인 스레드면 그 대기가 영원히 안 풀리는 데드락이었다. 기능 자체를 없애면서 이 버그도 같이 사라졌다.

⚠️ **폴더 대신 파일을 직접 열도록 변경 (2026-08-24):** `open_settings_folder`(폴더를 열어줌) → `open_settings_file`(`settings.json` 파일 자체를 OS 기본 프로그램으로 바로 염)로 변경 - `parent()`로 폴더 경로를 구하는 중간 단계를 없애고 `app.opener().open_path()`에 파일 경로를 직접 넘김. 한 클릭 덜 들도록.

⚠️ **버그 발견·수정 (2026-08-24):** `file_watch_debounce_ms`를 설정값으로 빼면서 발견 - `settings.json`에서 폴더를 지운 직후 그 폴더에 파일을 하나 쓰면(`index_worker_applies_settings_file_changes_live` 테스트가 정확히 이 순서), `notify`가 이미 큐에 넣어둔 그 생성 이벤트가 `apply_folder_diff`의 `unwatch()` 호출과 무관하게 그대로 살아남아 결국 색인돼버리는 경쟁 상태가 실제로 있었다 - `unwatch()`는 앞으로의 이벤트만 막고, 이미 채널에 들어온 이벤트를 되돌려 지우지는 않기 때문. 설정 파일 워처의 debounce를 3000ms→200ms로 줄이면서 타이밍이 바뀌어 이 경쟁이 매번 재현되는 쪽으로 굳어져 발견됨(전엔 우연히 안전한 순서로 풀렸을 뿐). 근본 수정: `run_index_worker`가 `folder_watcher`에서 받은 이벤트를 색인하기 직전, 그 경로가 **현재** `watched` 목록 아래에 있는지 다시 한번 필터링 - 타이밍에 의존하지 않는 결정적 수정.

⚠️ **설정값 완전성 재검토 (2026-08-24):** 폐기된 Settings Window mockup(`12_UI_Spec.md` C5)에 있던 항목들이 실제로 `settings.json`에 다 반영됐는지 전수 점검하고, 빠져 있던 것들을 추가했다 — `core::config::Config`에 `excluded_extensions`/`excluded_temp_patterns`(기존엔 고정 상수), `hotkey`(TASK-802 참조), `result_limit`(기존엔 프론트엔드 상수) 4개 필드 신설. `색인 초기화`는 값이 아니라 동작이라 트레이 메뉴 액션("Reset Index")으로 별도 구현(아래 Tray 섹션). `색인 DB 저장 위치`는 이미 확정된 배제 결정(`core/src/config.rs`의 `db_path` `#[serde(skip)]`)을 그대로 유지, `시작 시 자동 실행`은 새 의존성이 필요한 별도 기능이라 이번 범위에서 제외, `색인 스로틀링 파라미터`는 기존 비노출 결정 유지 - 자세한 표는 `12_UI_Spec.md` C5 참조.

⚠️ **`시작 시 자동 실행` 구현 (2026-08-23):** 사용자가 요청해 뒤늦게 추가 - `tauri-plugin-autostart` 의존성 + `Config::auto_start` (`bool`, 기본 `false`) 필드. `hotkey`와 같은 실시간 반영 패턴: `sync_autostart(app, enabled)`가 앱 시작 시와 `settings.json` 리로드(`auto_start`가 바뀔 때만) 양쪽에서 OS 로그인 항목을 값에 맞춰 등록/해제. 실패는 로그만 남기고 무시(OS가 거부해도 앱 시작을 막지 않음). 프론트엔드 IPC 커맨드는 없음 - `settings.json` 직접 편집만으로 켜고 끔, 다른 노출 없는 설정값들과 동일.

⚠️ **지원 포맷 확정 및 제외 규칙 축소 (2026-08-24, 이어서):** 검색 대상 포맷을 워드/엑셀/파워포인트/PDF/TXT/MD 6종으로 확정(구버전 `.doc`/`.xls`/`.ppt`는 범위 밖) - `core::extract::txt::TxtExtractor`가 `.md`도 `.txt`와 같은 방식(마크다운 문법 파싱 없이 원문 그대로)으로 처리하도록 확장. 그 김에 `excluded_extensions`(zip/7z/rar 차단 목록)를 완전히 제거 - `core::index::pipeline`이 등록된 `ContentExtractor` 중 매칭되는 게 없으면 이미 SKIP시키므로(고정된 지원 포맷 화이트리스트가 사실상의 필터), 별도 확장자 차단 목록은 무의미했다. `excluded_temp_patterns`는 그대로 유지 - `~$문서.docx`처럼 확장자는 지원 대상이어도 파일명 패턴으로만 걸러낼 수 있는 문제라 화이트리스트로 대체 불가(`01_KnowDesk_PRD.md` "기본 제외 규칙" 결정 참조).

⚠️ **`excluded_temp_patterns`도 설정값에서 제거 (2026-08-24, 이어서):** 패턴 목록(`~$`/`.tmp`/`.temp`/`.cache`) 자체가 고정된 값이라 사용자가 바꿀 이유가 없다는 판단 - `Config` 필드를 없애고 `core::scan::filter::check()`가 `core::config::DEFAULT_TEMP_PATTERNS` 상수를 직접 참조하도록 되돌렸다. 걸러내는 로직 자체(임시 파일 스킵)는 그대로, `settings.json`으로 노출만 안 할 뿐.

⚠️ **후속 조정 (2026-08-24, 이어서):** ①`result_limit`은 `0`을 무제한으로 해석하도록 `core::search::SearchRequest`/`SqliteSearchService`에 정규화 로직 추가(SQLite의 "음수 `LIMIT`은 무제한" 관례로 변환), 기본값도 무제한(`0`)으로 변경. ②하드코딩 값 전체 재검토에서 찾은 `file_watch_debounce_ms`(폴더 감시 debounce, 기존 3000ms 고정)를 설정값으로 추가. ③`settings.json` 자신을 감시하는 워처의 debounce는 설정값으로 빼지 않고 3000ms→200ms 내부 고정값으로만 낮춤 - 사용자가 튜닝할 대상이 아니라는 판단.

---

## Tray

TASK-801 Tray Manager

⚠️ **macOS 템플릿 아이콘 (2026-08-24):** `TrayIconBuilder`에 `.icon_as_template(true)` 추가 - macOS 전용(다른 OS에서는 no-op), 메뉴바가 라이트/다크 모드에 맞춰 아이콘 색을 자동으로 바꿔준다. 흑백 실루엣 + 알파 채널로 된 이미지여야 제대로 나오는데, 지금 `src-tauri/icons/32x32.png`는 아직 단색 사각형 플레이스홀더라 이 옵션을 켠 상태에선 메뉴바에 검은 사각형으로 보인다 - 실제 아이콘 이미지로 교체(`npx tauri icon <원본>` 또는 파일 직접 교체 후 재빌드)가 아직 남아있다.

⚠️ **트레이 전용 백그라운드 앱으로 확정 (2026-08-24):** 태스크바/Dock에 실행 상태로 뜨지 않고 트레이 아이콘만 남도록 함. macOS는 `app.set_activation_policy(tauri::ActivationPolicy::Accessory)`(Dock 아이콘·Cmd+Tab 전환창 둘 다 제외), Windows/Linux는 `tauri.conf.json` 검색창 설정의 `skipTaskbar: true`로 처리 - 창이 보일 때도 태스크바 버튼은 안 생김.

⚠️ **중복 실행 방지 (2026-08-24):** `tauri-plugin-single-instance` 도입, 빌더 체인 맨 앞에 등록(공식 권장 순서). 이미 실행 중일 때 두 번째로 실행하면 새 프로세스는 뜨지 않고, 기존 프로세스의 검색창을 보여주고 포커스만 준다(`show_search_window` - 좌클릭/단축키의 토글과 달리 항상 보여주기만 함, 이미 열려 있어도 숨기지 않음). 실제로 두 번 실행해서 프로세스가 하나만 남는 것 확인.

⚠️ **Reset Index 추가 (2026-08-24):** 트레이 우클릭 메뉴에 "Reset Index" 항목 신설 (`Settings` / 구분선 / `Reset Index` / 구분선 / `Quit`). 클릭 시 `tauri_plugin_dialog`의 non-blocking `.show()` 콜백으로 확인 다이얼로그를 띄우고(TASK-704 데드락의 원인이었던 `blocking_*` API는 쓰지 않음), 확인되면 채널로 색인 워커 스레드에 신호를 보내 `core::db::documents::DocumentRepository::reset_all`로 DB를 비운 뒤 감시 중인 모든 폴더를 처음부터 재스캔한다.

TASK-802 Hotkey Manager (창 사전 생성 + show/focus 방식 — P95 300ms 대응)

⚠️ **설정값으로 전환 (2026-08-24):** 하드코딩 상수(`DEFAULT_HOTKEY`, `src-tauri`)였던 걸 `core::config::Config::hotkey`로 옮겨 `settings.json`에서 바꿀 수 있게 했다. `settings.json` 변경 감지 시 색인 워커가 부르는 콜백(`run()`이 `spawn_index_worker`에 넘긴 `on_settings_reload`)이 이전 값을 `global_shortcut().unregister()`하고 새 값을 재등록한다 — 재시작 불필요. 이 콜백을 인젝션한 이유: `run_index_worker`/`spawn_index_worker`는 `AppHandle` 없이도 그대로 유닛 테스트되게 유지하기 위함(가짜 Tauri 앱을 안 만들어도 됨).

⚠️ **토글 동작 3단계로 변경 (2026-08-24):** `toggle_search_window`가 단순 `is_visible()` 토글이 아니라 `is_focused()`까지 같이 봐서 3단계로 동작한다 — 보이고+포커스 있음(숨김), 보이지만 포커스 없음(포커스만 다시 가져옴, 안 숨김), 숨겨짐(보이고 포커스). 트레이 좌클릭과 전역 단축키가 이 함수를 공유(`12_UI_Spec.md` C4 참조).

---

## Diagnostics

TASK-901 Statistics Service (완료, 2026-08-24)

⚠️ **`documents` 스키마 축소:** `index_status`/`demotion_reason`/`drm_status`/`retry_count`/`last_attempt_at`/`content_stored` 6개 컬럼 제거(`core/src/db/migrate.rs` MIGRATIONS v3, 상세 근거는 `04_Data_Model.md` 변경 이력 참조) - 어느 것도 실제 코드에서 읽거나 쓰인 적이 없었다. 이 통계 서비스(TASK-901)가 구현될 때 `demotion_reason`별 집계(`count_by_demotion_reason`, 제거됨)를 낼 계획이었다면 이제는 낼 수 없다 - 실패 사유 구분 자체가 필요 없다고 결정됐으므로, `index_tier`별 집계(`count_by_tier`, 유지됨)만으로 충분.

⚠️ **구현:** Settings Window가 없는 것과 같은 이유로 별도 통계 화면 대신 트레이 메뉴 액션으로 뺐다 (`Settings` / `Statistics` / 구분선 / `Reset Index` / 구분선 / `Quit`). 클릭하면 `src-tauri`의 `compute_stats`가 `knowdesk-cli stats`처럼 독립된 짧은 DB 연결을 열어(색인 워커 스레드와 조율 불필요한 단순 읾기) 전체 문서 수, Full/Meta 건수, DB 파일 크기(`std::fs::metadata`), 마지막 색인 시각(`DocumentRepository::last_indexed_at`, 신설)을 모아 `tauri_plugin_dialog`의 Info 다이얼로그로 보여준다. **Skip 건수는 포함하지 않음** - `core::index::pipeline::index_file`이 SKIP인 파일은 처음부터 `documents` 행 자체를 안 만들기 때문에 DB에서 조회할 수 있는 집계가 아니다(누적 스킵 카운터를 새로 영속화하는 건 이번 범위 밖으로 판단).

TASK-902 Log Export → **폐기 (2026-08-24)** - 불필요하다고 판단, 구현된 적 없음.

TASK-904 초기 색인 진행률 표시 (완료, 2026-08-24) — 온보딩 위저드 아님, 진행률 + "색인 중" 상태 문구만 (`KnowDesk_추가검토사항.md` E-2 참조)

⚠️ **구현:** `core::index::pipeline::IndexPipeline::index_directory_with_progress`(`index_directory`가 내부적으로 호출하는 기존 메서드는 그대로 유지, 새 오버로드만 추가) - `walker::scan`이 이미 전체 목록을 한 번에 반환하므로 총 개수를 미리 알 수 있어, 파일마다 `on_progress(done, total)` 콜백을 부른다. `apply_folder_diff`가 이 콜백으로 `Arc<Mutex<Option<{done, total}>>>` 공유 상태를 갱신 - 여러 폴더가 한 번에 "added"되는 경우(앱 시작 시 `watched_folders`를 전부 처음 스캔하는 경우가 정확히 이 상황)에도 폴더별로 0으로 리셋되지 않고 하나의 연속된 총합으로 보이도록 전체 폴더의 파일 수를 미리 합산. 스캔이 전부 끝나면 다시 `None`(idle)으로. `get_index_progress` 커맨드로 검색창이 폴링(테마처럼 포커스 시점에만 다시 읽는 게 아니라, 진행 중일 때는 1초 간격으로도 계속 재확인 - 설정값과 달리 창이 떠 있는 동안 계속 바뀌는 값이라서). 트레이 hover 텍스트가 아니라 검색창 상단 배너로 구현(`index.html`의 `#index-progress`) - 트레이 툴팁을 나중에 갱신하려면 트레이 핸들을 따로 보관해야 해서 더 복잡함.

---

# Phase D — Windows 이관

TASK-1001 Kiwi / PDFium Windows 바이너리 동봉 및 오프라인 초기화 경로 — 구현했으나 미검증 (2026-08-24, Windows 머신 없음). `tauri.windows.conf.json`(`bundle.resources`) + `src-tauri`의 Windows용 `set_bundled_native_lib_env_vars` 추가, macOS 버전과 동일한 패턴. 파일 경로는 `env.ps1`의 기존 가정(`pdfium.dll`은 `bin/` 아래 - 미확인, `kiwi.dll`은 `lib/` 아래 - 확인됨)을 그대로 따름. `06_Development_Roadmap.md` D1 참조

TASK-1002 Windows 경로 처리 (대소문자 정규화, 260자 초과, UNC 오프라인 처리)

TASK-1003 인스톨러 + 코드사이닝 — `bundle.targets` 부분만 구현하고 미검증 (2026-08-24). `tauri.windows.conf.json`에 `targets: ["msi", "nsis"]` 추가했으나 실제 빌드는 못 해봄. 코드사이닝은 여전히 미착수. `06_Development_Roadmap.md` D3 참조

TASK-1004 P95 성능 실측 및 튜닝

TASK-1005 DRM 적용률 실측 (O-4)

---

# Phase E — 단독 배포 도구

TASK-1101 `kdfind` — 사전 색인 없는 1회성 검색 CLI (완료, 2026-08-25). `knowdesk-cli`와 별개인 두 번째 바이너리(`cli/src/bin/find.rs`) — 폴더+검색어를 한 번에 받아 인메모리로 색인·검색하고 종료 시 아무것도 안 남긴다. 필터(`x:`/`p:`/`m>` 등)는 별도 플래그 없이 GUI와 동일하게 검색어 문자열에 그대로 섞어 쓴다. 단독 배포 대상이라 `KNOWDESK_*` 환경변수를 전혀 읽지 않고, 전용 설정 파일 `settings_cli.json`에서만 Kiwi/PDFium 네이티브 경로를 읽는다. 상세 설계·근거는 `docs/13_CLI_Tool.md`, 사용법은 `cli/README.md` 참조.
