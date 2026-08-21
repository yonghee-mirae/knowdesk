# KnowDesk 구현 계획

`CLAUDE.md`의 Mandatory Reading Order 2번 문서. PRD 다음으로 우선하며, 문서 전반의 C#/.NET 표기·단일 실행 파일·HMAC 조항을 개정한다.

## Context

`docs/`의 다른 문서들은 원래 **C#/.NET 전제로 작성**되어 있었다 — `interface IContentExtractor` C# 코드블록, "Solution 생성", `FileSystemWatcher`, DI 구성. 그러나 프로젝트 경로는 `src/rust/`이고, 개발 환경에는 `cargo`만 있으며 `dotnet`은 없다. 옆 프로젝트 `lightmark`가 Rust workspace + Tauri 2 + TS/Web Components 구조로 되어 있어 실제 작업 스택이 명확했다.

또한 PRD의 두 제약 — **단일 파일 배포**와 **HMAC 기반 비가역 토큰 저장** — 이 설계를 심하게 왜곡시키고 있었다. 논의 과정에서 두 제약 모두 폐기되었고, 그 결과 Kiwi 동봉·PDFium 사용·원문 저장이 모두 가능해지면서 설계가 크게 단순해졌다.

이 문서의 목적은 그 개정 내용을 확정 기록으로 남기고, **Linux에서 끝까지 검증 가능한 헤드리스 코어를 먼저 완성**하는 순서로 MVP를 구현하는 계획을 제시하는 것이다.

---

## 확정 사항 (결정 로그)

| # | 결정 | 근거 |
|---|---|---|
| 1 | **Rust + Tauri 2** | 경로·툴체인·lightmark 선례. 문서의 C# 인터페이스는 Rust trait으로 번역 |
| 2 | **단일 파일 배포 폐기** | 인스톨러 방식. Kiwi(~120MB)·PDFium 네이티브 동봉 가능해짐 |
| 3 | **HMAC 토큰 폐기** | prefix 검색 복원, `snippet()` 네이티브 동작, 재추출 불필요 |
| 4 | **원문 저장 (구성 A)** | 아래 근거. `DocumentStore` 뒤에 숨겨 압축 하이브리드(D)로 전환 가능 |
| 5 | **SQLite FTS5** | 검색 문법이 메타데이터 필터 지배적 → SQL 조인 이점. 1만 건은 FTS5에 부담 없음 |
| 6 | **Linux 우선 개발** | Windows 환경 없음. 검증 가능한 코어를 먼저 완성하고 Windows 종속은 최후로 |

**결정 4의 근거 — 비대칭성.** 실측 결과 1만 건 기준 DB는 200~700MB(문서당 텍스트 6~40KB 가정)로, 압축 하이브리드 대비 33~43% 클 뿐이다. 결정적인 것은 전환 비용이다:

- `원문 저장 → 하이브리드` = DB 마이그레이션만, **재색인 불필요**
- `원문 미저장 → 저장` = 1만 개 파일 **전량 재추출**, 수 시간

토큰화 규칙·Kiwi 버전·동의어 사전을 바꿀 때마다 재색인이 필요한데, 원문이 DB에 있으면 수십 초, 없으면 수 시간이다. 검색 품질 튜닝은 반복 작업이므로 이 차이가 개발 속도를 직접 좌우한다.

---

## 문서 단일 출처

문서 간 충돌이 반복되는 원인은 같은 내용이 여러 문서에 중복 기재되어 있기 때문이다. 각 주제의 소유 문서를 아래로 고정한다 — 다른 문서는 참조만 하고 내용을 복제하지 않는다.

| 주제 | 단일 출처 |
|---|---|
| 제품 요구사항·기능 정의 | `01_KnowDesk_PRD.md` |
| 기술 스택·아키텍처 결정 | `11_Implementation_Plan.md` (본 문서) |
| 검색 문법 | `05_Search_Language_v1.md` |
| DB 스키마 | `04_Data_Model.md` |
| 마일스톤·태스크 | `06_Development_Roadmap.md` / `07_Coding_Agent_Backlog.md` |
| 미결 항목(O-*)·리스크(R-*) | `KnowDesk_추가검토사항.md` |

`CLAUDE.md`의 우선순위 규칙(`01` → `11` → 나머지)은 그대로 유지한다.

---

## 기술 스택

| 레이어 | 크레이트 | 비고 |
|---|---|---|
| DB | `rusqlite` (bundled) | FTS5 포함 컴파일. 외부 SQLite 불필요 |
| 형태소 | `kiwi-rs` 2026.7.24 | LGPL-2.1+. **동적 라이브러리 + 모델 동봉 필요** |
| PDF | `pdfium-render` 0.9 | 한글 CID 폰트 견고성. `pdf-extract`는 폴백 |
| XLSX | `calamine` 0.36 | 순수 Rust |
| DOCX/PPTX | `zip` + `quick-xml` 0.41 | **`docx-rs`는 이름과 달리 writer라 부적합** |
| TXT 인코딩 | `encoding_rs` + `chardetng` | CP949/EUC-KR 대응 (아래 참조) |
| 파일 감시 | `notify` 8.2 (디바운스는 `-mini`/`-full` 없이 직접 구현) | FileSystemWatcher의 크로스플랫폼 대체 |
| UI/패키징 | Tauri 2 + TS/Web Components + Vite | lightmark 패턴 재사용. React/Vue 금지 |

`zip` 크레이트는 최신이 `9.0.0-pre3` 프리릴리스이므로 **stable 계열로 고정**할 것.

> **디바운스 직접 구현 (Phase B4, 2026-08-21):** `notify-debouncer-mini`로 처음 구현했다가 **무한 재색인 루프**를 실제로 재현했다. Linux inotify 백엔드는 `OPEN`/`ATTRIB`까지 기본으로 감시하는데, 색인 파이프라인이 파일을 읽는 것(해시 계산, 텍스트 추출) 자체가 `OPEN` 이벤트를 만들어 "읽음→이벤트→재색인→다시 읽음"이 끝없이 돈다. `notify-debouncer-full` 소스도 확인했는데 `EventKind::Other`만 걸러내고 `Access`/`Modify(Metadata)`는 그대로 통과시켜 동일한 문제가 있다. 결론: 이 문제는 어떤 디바운서 크레이트를 쓰든 피할 수 없고(둘 다 원시 이벤트 필터링 지점을 안 열어줌), 원시 `notify::Event`를 직접 받아 `EventKind`로 필터링한 뒤 직접 디바운스해야 한다(`core/src/index/watcher.rs`). rename 전용 추적(`-full`이 제공)은 문서 식별이 내용 해시 기준이라 필요 없다.

### 문서에 없으나 반드시 필요한 것

**TXT 인코딩 감지.** 사내 TXT 문서 상당수가 UTF-8이 아니라 **CP949/EUC-KR**이다. 감지 없이 읽으면 한글이 깨진 채로 색인되는데, 이는 "검색이 안 된다"가 아니라 "조용히 틀린 색인이 쌓인다"는 형태로 나타나 발견이 매우 늦다. `chardetng`로 감지하고 `encoding_rs`로 디코딩한다.

**Kiwi 오프라인 초기화.** `kiwi-rs`의 `Kiwi::init()`은 GitHub 릴리스에서 라이브러리·모델을 **자동 다운로드**한다. 망분리 환경에서 동작하지 않으므로 `Kiwi::from_config()`로 동봉 경로를 명시하는 경로만 사용한다.

- `kiwi_win_x64_v0.22.2.zip` / `kiwi_lnx_x86_64_v0.22.2.tgz`
- `kiwi_model_v0.22.2_base.tgz`

> **버전 정정 (Phase B2, 2026-08-21):** 원래 명시했던 `v0.23.2`는 `kiwi-rs 2026.7.24`와 ABI가 맞지 않아 **세그폴트**한다. v0.23.2의 `kiwi_analyze_option_t`에 `typo_transformer`/`typo_threshold` 필드가 추가되었는데, `kiwi-rs 2026.7.24`의 FFI 구조체는 이를 모른 채 옛 5필드 레이아웃으로 값을 전달해 네이티브 코드가 오타 교정기 포인터 자리를 쓰레기 값으로 읽는다. 실제로 v0.23.2 라이브러리+모델로 `Kiwi::from_config` → `tokenize()`를 호출하면 `PreparedTypoTransformer::generateGraph` 안에서 죽는 것을 gdb로 확인했다. `v0.22.2`는 해당 필드가 없는 구버전 구조체와 맞아 정상 동작한다(실제 다운로드 후 한글 형태소 분리까지 검증). CONG 모델을 쓰므로 `build_options`는 `KIWI_BUILD_DEFAULT_WITH_CONG`을 명시해야 한다(`kiwi-cli --model-type cong`과 동일). 라이브러리/모델 경로는 `KNOWDESK_KIWI_LIB_PATH`(라이브러리 파일)·`KNOWDESK_KIWI_MODEL_DIR`(모델 디렉터리, 예: `models/cong/base`) 환경 변수로 지정한다.

> **토크나이저 역할 재설계 (Phase B2, 2026-08-21):** 원래는 bigram/Kiwi가 "택일"(둘 중 하나로 색인)이었으나, bigram은 항상 실행되는 **기본** 토크나이저(`content_fts.morph`), Kiwi는 가능할 때만 추가로 붙는 **보조** 토크나이저(`content_fts.morph_kiwi`, 새 컬럼)로 재설계했다. 검색어 분석도 새로 추가했는데, **Kiwi만** 적용한다 — bigram은 색인에서만 의미가 있고 검색어를 분석해도 짧은 음절 조각(예: "다" 같은 흔한 어미) 때문에 정밀도만 떨어뜨릴 뿐 회귀도 없고 이득도 없다(bigram 검색어 분석은 원래 존재한 적이 없었으므로, 안 넣는 것은 현재 동작 유지일 뿐 손실이 아니다). Kiwi 검색어 분석은 조사/어미 등 순수 문법 형태소를 제외한 의미 형태소만 남기고(`KiwiTokenizer::tokenize`의 세종 품사 태그 필터), 원래 검색어를 **교체가 아니라 OR로 추가**한다 — 문맥 없는 단독 검색어는 Kiwi도 오분석할 수 있어서(예: "이사회" 단독 입력 시 "이(관형사)+사회"로 잘못 쪼개짐, 실제 확인함) 리터럴을 안전망으로 남겨야 한다. 검색 결과는 리터럴로 걸렸으면 "정확 일치", 확장으로만 걸렸으면 "형태소 분석"으로 구분해 표시한다.

**파일 크기 상한.** PRD 기준 **기본 50MB 초과 파일은 SKIP** (사용자 설정 가능). Phase A3 `FileFilter`에 반영한다.

---

## 크레이트 구조

```
knowdesk/
├── Cargo.toml              # workspace
├── core/                   # Tauri를 모름. 전 비즈니스 로직.
│   ├── config.rs
│   ├── db/      schema.rs migrate.rs documents.rs search_repo.rs
│   ├── scan/    walker.rs filter.rs hash.rs
│   ├── extract/ mod.rs(trait) txt.rs ooxml.rs xlsx.rs pdf.rs
│   ├── nlp/     mod.rs(trait) kiwi.rs bigram.rs synonym.rs
│   ├── index/   pipeline.rs queue.rs watcher.rs
│   └── search/  parser.rs service.rs rank.rs snippet.rs
├── cli/                    # 헤드리스 검증 하니스 (Linux 개발의 핵심)
├── src-tauri/               # 트레이·전역단축키·IPC만
└── frontend/                # TS + Web Components + Vite
```

lightmark의 규칙을 그대로 따른다 — **`core`는 Tauri를 절대 참조하지 않는다.** `src-tauri`는 OS 통합과 IPC만 담당한다.

`cli` 크레이트가 Linux 우선 개발의 핵심 장치다. UI 없이 `index` / `search` / `stats` / `bench` 서브커맨드로 파이프라인 전체를 자동 테스트할 수 있게 한다.

### 핵심 trait

```rust
// extract/mod.rs — 문서의 IContentExtractor 대응
pub trait ContentExtractor {
    fn supports(&self, ext: &str) -> bool;
    fn extract(&self, path: &Path) -> Result<Extracted, ExtractError>;
}

// nlp/mod.rs — Kiwi 교체 가능하게
pub trait Tokenizer {
    fn tokenize(&self, text: &str) -> Vec<Token>;
}

// db/ — 원문 저장 방식을 감춤 (구성 A → D 전환용)
pub trait DocumentStore {
    fn put_body(&self, doc: DocId, text: &str) -> Result<()>;
    fn get_body(&self, doc: DocId) -> Result<Option<String>>;
}
```

세부 시그니처는 `08_API_Contracts.md`, 계층 배치는 `03_Architecture.md` 참조.

---

## 데이터 모델 — 반영 완료

`04_Data_Model.md`에 이미 반영했다. 요지: PRD 7장 `DocumentID = SHA256(Content)` + "경로는 별도 관리"에 맞춰 `documents`(내용 기준) / `paths`(경로 기준, 1:N)로 분리했고, `token_fts`는 제거하여 `content_fts(body, morph)` 두 컬럼으로 합쳤다. 부수 효과로 F-06(최신 버전 탐지)이 별도 유사도 연산 없이 스키마만으로 상당 부분 충족된다.

---

## 마일스톤

`06_Development_Roadmap.md` / `07_Coding_Agent_Backlog.md`에 이미 Phase A~D로 반영했다. 요지만 기록한다.

### Phase A — Walking Skeleton (Linux 전량 검증 가능)

workspace/config/logging/cli 뼈대 → SQLite 스키마+Repository → 폴더 스캔+50MB 상한 필터+SHA256 → **TXT 추출+인코딩 감지** → Tokenizer trait+bigram → 색인 파이프라인(FULL/META/SKIP+상태머신) → Query Parser+Search+bm25+snippet.

**마이그레이션 롤백(down) 필요 여부는 미결이다** (`KnowDesk_추가검토사항.md` C-3). 확정 전까지는 up 마이그레이션 테스트만 작성한다.

**완료 기준:** `cli index ./samples && cli search "채권 발행"`이 스니펫과 함께 결과를 반환한다. 이 시점에 제품의 핵심 가치가 증명된다.

### Phase B — 실사용 가능한 코어

XLSX→DOCX/PPTX→**PDF(pdfium)** 순 추출 확장 → **Kiwi 연동**(오프라인 `from_config`) → 동의어 사전 → `notify` 감시+디바운스+증분 색인 → **벤치마크 하니스**(`cli bench`)로 실제 문서 기반 DB 크기 재측정.

디바운스는 선택이 아니라 필수다 — Office가 저장할 때 임시 파일 생성 → 원본 삭제 → 이름 변경 폭풍이 발생해서, 그대로 받으면 같은 문서를 여러 번 재색인한다.

### Phase C — UI (Linux에서 대부분 검증 가능)

Tauri 셸+검색창+결과목록 → 프리뷰+하이라이트+스니펫 → 파일/폴더 열기·경로 복사(전부 키보드만으로) → 트레이+전역 단축키 → 설정+통계/진단.

**검색창 호출 P95 300ms 달성 방법:** 웹뷰 창을 앱 시작 시 미리 생성해 두고 숨긴 뒤, 단축키에는 show+focus만 시킨다. 콜드 스타트로는 300ms를 맞출 수 없다.

Tauri 플러그인(`tauri-plugin-global-shortcut`, 트레이)은 Linux에서도 동작하므로 **기능 검증은 가능하다.** 다만 Windows에서의 실제 거동은 확인할 수 없다.

### Phase D — Windows 이관 (여기서 처음 Windows 필요)

Kiwi·PDFium Windows 바이너리 동봉 → Windows 경로 처리 → 인스톨러+코드사이닝 → 성능 P95 실측 → **DRM 적용률 실측(O-4)** → Phase 2 선행 여부 판단.

---

## Linux에서 검증 불가 — Phase D 필수 확인 목록

이 항목들은 Linux에서 **틀린 채로 통과할 수 있으므로** 명시적으로 남긴다.

1. **경로 대소문자** — Windows는 무시, Linux는 구분. `paths` 테이블 중복 판정이 Windows에서만 깨진다. 정규화 로직을 넣되 Linux에서는 검증되지 않는다.
2. **260자 초과 경로** — 리서치 폴더 깊이에서 현실적으로 발생. `\\?\` 접두 처리 필요.
3. **UNC 네트워크 드라이브** — 오프라인 시 삭제로 오판하면 VPN 끊길 때마다 색인이 소실된다 (`KnowDesk_추가검토사항.md` D-1이 지적한 이슈). 보류 처리로 설계하되 실환경 확인 필요.
4. **EDR 오탐** — 전역 단축키 후킹이 키로거로(R-4), 상주 프로세스가 정책 위반으로(R-5) 차단될 수 있다. O-7·O-8 미확인 상태.
5. **DRM 문서 실거동** — O-4가 미실측이며, PRD도 이것을 최우선 항목으로 지목한다.

---

## 검증 방법

**단위·통합 테스트**
```bash
cargo test --workspace
```
포맷별 추출은 `core/tests/fixtures/`에 소형 샘플(TXT 3종 인코딩, DOCX, PPTX, XLSX, PDF)을 두고 기대 텍스트와 대조하는 골든 테스트로 검증한다.

**파이프라인 end-to-end (Phase A 완료 기준)**
```bash
cargo run -p cli -- index ./samples
cargo run -p cli -- search "채권 발행"
cargo run -p cli -- search 'ext:pdf path:리서치 modified>2026-01-01'
cargo run -p cli -- stats     # FULL/META/SKIP 건수, 강등 사유별 집계
```

**성능·용량 실측 (Phase B5)**
```bash
cargo run -p cli --release -- bench --corpus <실제문서경로>
```
색인 처리량(건/초), 검색 P95, DB 크기, 문서당 추출 텍스트 비율을 출력한다. PRD 성공 기준(검색 P95 1초, 창 호출 300ms, 유휴 CPU 1%, 메모리 200MB)은 측정 조건이 정의되어 있지 않으므로(`KnowDesk_추가검토사항.md` E-5), 이 하니스가 그 정의를 겸한다.

**UI (Phase C)**
```bash
npm run dev          # 브라우저 단독 모드 — lightmark 규칙 계승
npm run tauri dev    # Tauri 모드
```

---

## 열린 항목

계획 실행을 막지는 않으나 Phase D 전에 답이 필요하다. 상세는 `KnowDesk_추가검토사항.md`.

- **O-4 DRM 적용률** — PRD가 최우선으로 지목. 이 값이 높으면 Phase 2를 앞당겨야 한다
- **O-5 색인 DB 보안 검토** — 원문 저장을 택했으므로 DB 암호화(SQLCipher) 필요 여부가 여기서 결정된다. `DocumentStore` 추상화 덕분에 나중에 붙여도 재색인은 불필요하다
- **O-7 / O-8** — 전역 단축키·상주 프로세스 정책. Phase C4/D3 진행 전 확인
- **C-3 마이그레이션 롤백** — up-only로 할지 down도 만들지 미결. D-4에서 결정
