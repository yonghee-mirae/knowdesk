# `kdfind` — 사전 색인 없는 1회성 검색 CLI

`knowdesk-cli`(기존, 헤드리스 검증 하니스)와 별개로 만든 두 번째 바이너리(`cli/src/bin/find.rs`)에 대한 작업 계획·설계 결정 기록이다. 코드는 이 문서가 아니라 `cli/README.md`(사용법)를 따라 확인한다.

---

## 배경

`knowdesk-cli`는 `index`(폴더 → DB 파일에 색인) 후에야 `search`가 가능하다. 검색 한 번 하려고 DB 파일을 먼저 만들어 둬야 하고, 그 파일이 디스크에 남는다. 그리고 CLI는 실행할 때마다 새 프로세스이므로 "index 명령으로 인메모리 DB에 색인 → 다음 search 명령이 그걸 조회"는 애초에 불가능하다(프로세스가 끝나면 인메모리 DB도 함께 사라진다).

그래서 **폴더와 검색어를 한 번에 받아, 그 자리에서 인메모리로 색인하고 결과를 출력한 뒤 아무것도 남기지 않고 끝나는** 별도 바이너리 `kdfind`를 만들었다. GUI 앱과도, `knowdesk-cli`와도 별개로 단독 배포할 예정이다.

`knowdesk-cli`(index/search/stats/watch/bench)의 동작은 이 작업으로 전혀 바뀌지 않는다 — 역할이 다른 두 번째 도구를 추가한 것뿐이다.

| | `knowdesk-cli` | `kdfind` |
|---|---|---|
| 목적 | 코어 파이프라인 헤드리스 검증 (index/search/stats/watch/bench) | 사전 색인 없는 1회성 검색 |
| 색인 DB | 파일에 영구 저장 (`--db`) | 인메모리, 실행 종료 시 소멸 |
| 네이티브 라이브러리(Kiwi/PDFium) 경로 | `KNOWDESK_*` 환경변수 | 전용 설정 파일(`settings_cli.json`)만 — 환경변수 안 읽음 |
| 배포 | 안 함 (개발 중에만 `cargo run`) | 단독 배포 |

---

## 설계 결정과 근거

### 인메모리 DB, 색인·검색을 한 명령으로

`core::db::Db::open_in_memory()`(이미 존재)로 SQLite 인메모리 DB를 열고, 같은 프로세스 안에서 `IndexPipeline::index_directory` → `SqliteSearchService::search`를 순서대로 실행한다. 위 "배경"에서 설명한 대로 프로세스 분리 자체가 안 되므로 이 둘을 분리된 서브커맨드로 둘 이유가 없다.

### 필터 전용 플래그를 두지 않음

`x:pdf`, `p:리서치`, `m>2026-01-01` 같은 필터는 별도 CLI 플래그로 받지 않고, GUI 검색창에 타이핑하는 것과 동일하게 검색어 문자열 안에 그대로 섞어 쓴다. `core::search::parser::parse()`/`parse_filename()`이 이미 필터 토큰을 문자열 어디서든 추출하도록 돼 있어서(`docs/05_Search_Language_v1.md`), 새 파싱 로직이 전혀 필요 없다 — `kdfind`는 인자들을 공백으로 이어붙여 그대로 넘기기만 한다.

검색어 인자는 clap의 trailing var-arg로 받는다. 셸이 이미 공백으로 토큰을 나눠서 넘겨주고 `kdfind`가 다시 공백으로 합치기 때문에, `채권 AND 발행`처럼 연산자를 포함한 검색어도 따옴표 없이 그대로 동작한다. 단, `"채권 발행"`처럼 구(phrase) 검색을 위한 큰따옴표는 셸이 그 자체를 그룹핑 문법으로 소비해버리므로, 리터럴 `"` 문자가 살아서 전달되도록 셸에서 한 번 더 감싸야 한다(예: `'"채권 발행"'`) — `kdfind`가 해결할 수 있는 문제가 아니라 셸 사용법 차원의 문제라, `--help`(`after_help`)에 예시로 안내한다.

### 환경변수를 배제하고 `settings_cli.json`만 쓰는 이유

`knowdesk-cli`는 Kiwi/PDFium 네이티브 라이브러리 경로를 `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`/`KNOWDESK_PDFIUM_LIB_DIR` 환경변수로만 받는다(개발 중 `source ./env`로 설정). 이 방식은 저장소 안에서 개발할 때는 편하지만, `kdfind`처럼 **바이너리 하나만 뚝 떼어 다른 PC에 배포**하는 경우에는 매번 셸 프로파일에 환경변수를 심어야 해서 배포 대상 도구에 맞지 않는다.

그래서 `kdfind`는 `KNOWDESK_*` 환경변수를 일절 읽지 않고, 전용 설정 파일 `settings_cli.json`에서만 이 경로들을 읽는다. GUI 앱의 `settings.json`을 그대로 재사용하지 않은 이유는, `Config`(GUI용)가 `watched_folders`/`theme`/`hotkey`처럼 `kdfind`와 무관한 필드투성이인 데다, Kiwi/PDFium 경로 필드 자체가 없기 때문이다(GUI는 이 경로도 환경변수로만 받는다). `kdfind` 전용의 작고 독립된 스키마가 더 맞다.

---

## `settings_cli.json`

**위치:** GUI `settings.json`과 같은 앱 데이터 폴더 — `core::config::app_data_dir()/settings_cli.json`.

| OS | 경로 |
|---|---|
| Linux | `~/.local/share/KnowDesk/settings_cli.json` (`$XDG_DATA_HOME` 설정 시 그쪽) |
| macOS | `~/Library/Application Support/KnowDesk/settings_cli.json` |
| Windows | `%LOCALAPPDATA%\KnowDesk\settings_cli.json` |

경로를 지정하는 플래그·환경변수 오버라이드는 없다 — 항상 이 고정 위치 하나뿐이다. 파일이 없으면 첫 실행 시 기본값으로 자동 생성한다(GUI가 `settings.json`에 하는 것과 동일한 패턴).

```json
{
  "enable_morphological_analysis": false,
  "kiwi_lib_path": null,
  "kiwi_model_dir": null,
  "pdfium_lib_dir": null
}
```

- `enable_morphological_analysis`가 `false`거나 `kiwi_lib_path`/`kiwi_model_dir` 중 하나라도 비어 있으면 Kiwi 로드 시도 자체를 하지 않고 bigram만 쓴다 — 의도된 기본 상태이므로 경고를 찍지 않는다. 켜져 있는데 두 경로 중 하나가 비어 있거나 로드에 실패하면 그때만 stderr 경고.
- `pdfium_lib_dir`가 비어 있으면 PDF는 메타(파일명만) 색인되고, 본문 검색은 되지 않는다.
- `max_file_size_mb`는 넣지 않았다 — 요청 범위 밖. `Config::default()`의 50MB(core 기본값)를 그대로 쓴다.

---

## 구현 요약

- `core::config::app_data_dir()` — GUI(`src-tauri`)가 갖고 있던 것을 `core`로 옮겨 `cli`와 공유(`03_Architecture.md`의 "core는 Tauri를 모른다" 규칙은 그대로 유지 — 순수 경로 계산일 뿐).
- `core::extract::pdf::PdfExtractor::set_lib_dir()` — 호출자가 PDFium 경로를 명시적으로 넘기면 `KNOWDESK_PDFIUM_LIB_DIR` 환경변수 조회를 완전히 건너뛴다. 호출하지 않으면(`knowdesk-cli`/GUI는 호출하지 않음) 기존 동작 그대로.
- `cli/src/lib.rs` — `support`(두 바이너리가 공유하는 `default_extractors()`)와 `cli_config`(`CliConfig`, `settings_cli.json` 로드/저장)를 담은 라이브러리 타깃. `src/bin/` 아래 바이너리는 `src/` 아래 파일을 직접 `mod`할 수 없어서 필요.
- `cli/src/bin/find.rs` — `kdfind` 본체.

---

## 검증

1. `cargo build -p knowdesk-cli` — `cli`/`kdfind` 두 바이너리 모두 빌드
2. `cargo test -p knowdesk-core -p knowdesk-cli` — 기존 테스트 전부 통과, `cli_config` 단위 테스트(파일 없을 때 기본값 / 라운드트립 / 일부 필드만 있는 JSON), `cli/tests/find.rs` 통합 테스트(키워드 검색·확장자 필터·파일명 모드·limit·최초 실행 시 설정 파일 자동 생성)
3. 수동 확인 — `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`/`KNOWDESK_PDFIUM_LIB_DIR`를 일부러 존재하지 않는 값으로 설정해두고, `settings_cli.json`에 실제 경로를 채운 뒤 `kdfind`가 그 설정 파일만으로 Kiwi/PDF를 정상 동작시키는지 확인 — 환경변수는 완전히 무시됨을 재확인.

---

## 알려진 트레이드오프 (의도된 동작)

- 실행할 때마다 대상 폴더 전체를 다시 스캔·추출·토크나이즈한다. "사전 색인 DB 없음"이 요구사항이므로 당연한 대가 — 같은 폴더를 반복 검색하면 매번 비용이 든다.
- Kiwi를 켜면 실행마다 로드 비용(실측 ~824MB RSS, `06_Development_Roadmap.md` S-2)이 든다. 기본값이 꺼짐인 이유.
