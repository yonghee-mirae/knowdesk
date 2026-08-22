# KnowDesk

Your Knowledge, On Your Desk.

증권사 임직원이 개인 PC에 쌓아둔 업무 매뉴얼·규정집·리서치 자료·회의자료 등을 전역 단축키로 즉시 호출해 본문까지 검색하는 로컬 검색 플랫폼이다. 인터넷 연결·외부 API·중앙 서버 없이 개인 PC 안에서만 동작한다.

---

## 빌드 및 자동 테스트

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

---

## 수동 테스트

지금까지 구현된 범위(TXT/XLSX/DOCX/PPTX/PDF 추출 + 색인 + 검색)를 한 번에 확인할 수 있는 샘플 생성기가 있다. 생성물(`samples/`, `*.db`)은 매번 재생성 가능하므로 git에는 커밋하지 않는다 (`.gitignore` 참조).

PDF 추출은 네이티브 **libpdfium** 동적 라이브러리가 있어야 실제로 동작한다. 없으면 오류가 아니라 META(본문 미추출)로 조용히 강등되므로, PDF까지 검증하려면 `KNOWDESK_PDFIUM_LIB_DIR` 환경 변수로 라이브러리가 있는 디렉터리를 지정한다 (예: [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) 릴리스에서 `pdfium-linux-x64.tgz`를 받아 압축을 풀면 나오는 `lib/` 경로). Windows 배포판에서는 이 경로 지정이 필요 없다 — 인스톨러가 실행 파일 옆에 동봉한다 (`03_Architecture.md`).

형태소 분석은 bigram이 항상 채우는 기본 토크나이저이고, Kiwi는 네이티브 라이브러리·모델이 있을 때만 추가로 붙는 보조 토크나이저다. 둘 중 하나라도 없으면 (오류 없이) Kiwi 없이 bigram만 쓴다. Kiwi까지 검증하려면 [bab2min/Kiwi](https://github.com/bab2min/Kiwi/releases) 릴리스에서 **`v0.22.2`** (⚠️ `v0.23.2`는 `kiwi-rs 2026.7.24`와 ABI가 맞지 않아 세그폴트한다 — `11_Implementation_Plan.md` 참조) 의 `kiwi_lnx_x86_64_v0.22.2.tgz`(라이브러리)와 `kiwi_model_v0.22.2_base.tgz`(모델)를 받아 압축을 풀고, `KNOWDESK_KIWI_LIB_PATH`(예: `lib/libkiwi.so`)와 `KNOWDESK_KIWI_MODEL_DIR`(예: `models/cong/base`) 환경 변수로 각각 지정한다.

> **주의:** `index`는 동일한 내용(SHA256)의 문서가 이미 DB에 있으면 재추출하지 않고 기존 계층을 그대로 쓴다. `KNOWDESK_PDFIUM_LIB_DIR`/`KNOWDESK_KIWI_LIB_PATH` 등 환경 변수를 바꿔서 다시 검증할 때는 `rm -f ./samples.db`로 DB를 지우고 다시 색인해야 한다 — 지우지 않으면 이전 강등 결과가 그대로 남아 있다.

```bash
# 1. 샘플 폴더 생성 (./samples)
cargo run -p knowdesk-core --example gen_samples

# 2. 색인 (PDF까지 검증하려면 KNOWDESK_PDFIUM_LIB_DIR=<lib 경로>,
#    Kiwi까지 검증하려면 KNOWDESK_KIWI_LIB_PATH=<so 경로> KNOWDESK_KIWI_MODEL_DIR=<모델 경로> 접두)
cargo run -p knowdesk-cli -- --db ./samples.db index ./samples

# 3. 검색 (스니펫과 함께 결과가 나오면 정상)
cargo run -p knowdesk-cli -- --db ./samples.db search "채권 발행"

# 4. 통계
cargo run -p knowdesk-cli -- --db ./samples.db stats
```

### 샘플 구성

| 파일 | 목적 |
|---|---|
| `규정.txt` | 기본 UTF-8 TXT |
| `회의록_EUCKR.txt` | 인코딩 자동 감지 (EUC-KR) |
| `무관.txt` | 검색어와 무관한 문서 (오탐 없는지 확인용) |
| `공사보고서.txt` | Kiwi 연동 확인용 — "지었다"(짓다의 ㅅ 불규칙 활용)를 어간 "짓"으로도, 사전형 검색어 "짓다"로도 찾을 수 있는지 확인 |
| `실적표.xlsx` | XLSX 추출 |
| `이사회결의.docx` | DOCX 추출 |
| `발표자료.pptx` | PPTX 추출 (슬라이드 2개, 순서 확인용) |
| `검토의견.pdf` | PDF 추출 — 한글 CID 폰트가 실제로 임베딩된 PDF (LibreOffice headless로 생성) |
| `보관용.zip` | 압축 파일 → SKIP |
| `~$규정.txt` | 임시 파일 → SKIP |
| `손상.pdf` | 손상된 PDF → META(PARSE_FAIL) |

기대 결과: `KNOWDESK_PDFIUM_LIB_DIR` 미지정 시 11건 중 7건 FULL, 2건 META(`검토의견.pdf`+`손상.pdf`, 둘 다 PARSE_FAIL), 2건 SKIP. 지정 시 `검토의견.pdf`도 FULL로 올라가 8건 FULL, 1건 META(`손상.pdf`만), 2건 SKIP.

Kiwi 연동 여부는 색인 건수로는 구분되지 않는다(형태소 분석은 tier에 영향을 주지 않음) — 대신 검색으로 직접 확인한다:

```bash
cargo run -p knowdesk-cli -- --db ./samples.db search "짓"    # 어간 — 색인 시점 형태소 분석 확인
cargo run -p knowdesk-cli -- --db ./samples.db search "짓다"  # 사전형 — 검색어 형태소 분석(확장) 확인
```

Kiwi가 실제로 동작 중이면 둘 다 `공사보고서.txt`를 찾고, 둘 다 `[morphological match]`가 붙는다 — "지었다"(지/었/다)는 "짓"이라는 글자를 원문 어디에도 포함하지 않으므로, 검색어가 "짓"이든("짓다"를 거치지 않고 이미 어간 그대로 입력) "짓다"든(검색어 확장을 거쳐 어간 "짓"이 됨) 결국 Kiwi의 형태소 분석(색인 시점이든 검색어 확장 시점이든)이 있어야만 찾아지는 건 마찬가지다. `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`가 없어 bigram만 쓴다면 둘 다 `No results`다.

검색 결과에는 각 히트가 어떻게 걸렸는지 태그가 붙는다: 원문에 검색어(또는 형태소 분석이 찾아낸 어간)가 문자 그대로 존재하면 `[exact match]`, 원문에 없고 Kiwi의 형태소 분석으로만 찾아졌으면(불규칙 활용 등으로 표면형이 아예 다른 경우) `[morphological match]`. 예를 들어 `search "채권 발행"`은 원문에 "채권"과 "발행"이 그대로 있으므로 `[exact match]`가 나오는지 확인해보면 된다 — 평범한 검색어는 확장 기능이 있어도 동작이 그대로여야 한다. (`[exact match]`/`[morphological match]` 판정은 검색어가 확장됐는지가 아니라, 실제로 원문에 문자 그대로 있는지로 결정된다 — 2026-08-22 수정. 예전엔 "짓"처럼 확장이 필요 없는 검색어는 무조건 `[exact match]`로 잘못 표시됐다.)

### CLI 서브커맨드

| 명령 | 설명 |
|---|---|
| `index <경로>` | 폴더를 스캔해 색인 |
| `search <검색어> [--mode filename\|content] [--limit N]` | 검색 (기본: content 모드) |
| `stats` | 계층별(FULL/META/SKIP) 색인 건수 |
| `watch <경로> [--debounce-ms N]` | 폴더를 계속 감시하며 변경을 즉시 색인 (Ctrl+C로 종료, 기본 디바운스 3000ms) |
| `bench` | 벤치마크 (Phase B5에서 구현 예정, 현재 스텁) |

`watch`는 먼저 전체 스캔을 한 번 하고(감시가 꺼져 있던 동안의 변경 반영), 그 뒤로는 생성·수정·삭제만 반영한다. 파일 삭제 시 그 문서를 참조하는 다른 경로가 더 없으면 색인에서도 완전히 지운다(orphan 정리). 예:

```bash
cargo run -p knowdesk-cli -- --db ./watch.db watch ./samples &
echo "새 문서" > ./samples/새문서.txt   # 잠시 후 자동 색인됨
rm ./samples/새문서.txt                # 잠시 후 색인에서도 사라짐
```

검색 필터는 `docs/05_Search_Language_v1.md` 문법을 그대로 따른다: `x:pdf`, `p:리서치`, `m>2026-01-01`, `m<2026-08-01`, `m=2026-08-10` 등을 검색어에 함께 넣으면 된다.

`--db` 옵션 없이 실행하면 현재 디렉터리에 `knowdesk.db`가 생성되므로, 테스트할 땐 `--db` 경로를 지정해 격리하는 걸 권장한다.

---

## Phase C — 실제 UI (Tauri)

`src-tauri/` + `frontend/`가 Phase C의 실제 검색창 구현이다(`docs/12_UI_Spec.md` C1 검색창 + 결과 리스트 + 프리뷰). 브라우저 프로토타입(`docs/06_Development_Roadmap.md` Phase C 착수 전 만든 목업)에서 검증한 상호작용을 실제 `knowdesk-core` 검색과 연결한 것이다.

```bash
npm install
npm --prefix frontend install

# CLI로 이미 색인해 둔 DB를 그대로 붙여서 확인하려면 KNOWDESK_DB_PATH 지정
# (없으면 OS별 앱 데이터 폴더의 knowdesk.db를 새로 만든다 — 아직 비어 있음, 폴더 색인 트리거는 TASK-704 이후 범위)
# 반드시 절대경로로 지정할 것 — `tauri dev`가 내부적으로 src-tauri/를 작업 디렉터리로 두고
# 실행하므로, 여기서 상대경로(./samples.db)를 쓰면 저장소 루트가 아니라
# src-tauri/samples.db라는 새 빈 DB가 조용히 생성된다 (검색 결과 0건으로만 나타나고 에러는 없음).
KNOWDESK_DB_PATH="$(pwd)/samples.db" npm run tauri dev
```

키보드: `↑`/`↓` 이동, `Enter` 열기, `Ctrl+Enter` 폴더 열기, `Ctrl+C` 경로 복사, `Ctrl+Tab`/`Ctrl+1`/`Ctrl+2` 내용·파일명 모드 전환, `Esc` 창 숨김.

프로토타입에서 확정된 요소 중 실제 UI에 반영된 것: 검색 문법 도움말 패널(검색어가 비어 있을 때 결과 리스트 자리에 전체 폭으로 표시), 다크모드 토글 버튼, 결과 항목의 경로(meta-line) 표시, "결과 없음" 2단 안내 문구, 창을 둥근 모서리로 띄우는 플로팅 카드 룩(투명 창 + CSS `box-shadow` — macOS 네이티브 창 그림자는 웹뷰 투명도와 무관하게 창 프레임 전체를 사각형으로 그리므로 꺼두고 대체했다, `frontend/src/styles/layout.css` 참조).

프로토타입과 의도적으로 다른 점: 폰트 — PRD의 "인터넷 연결 없이 동작" 원칙 때문에 프로토타입의 Google Fonts(IBM Plex) 대신 OS 기본 한글 폰트를 쓴다.

아직 반영 안 된 것(백엔드가 없어서 채울 데이터가 없음): 설정(⚙) 버튼은 자리만 있고 비활성 상태(TASK-704), 하단 색인 상태 문구는 아직 없음(TASK-901/904 — 실제 색인 진행 데이터가 있어야 하므로 가짜 숫자를 넣지 않았다). 트레이·전역 단축키(TASK-801/802 — 그때까지는 `npm run tauri dev`로 직접 띄워야 창이 보인다).
