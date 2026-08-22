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

Kiwi가 실제로 동작 중이면 둘 다 `공사보고서.txt`를 찾는다 — "짓"은 `morph_kiwi`에 이미 그대로 있는 토큰이라 `[exact match]`, "짓다"는 검색어 확장을 거쳐야 찾아지므로 `[morphological match]`가 붙는다. `KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`가 없어 bigram만 쓴다면 둘 다 `No results`다.

검색 결과에는 각 히트가 어떻게 걸렸는지 태그가 붙는다: 리터럴(body/bigram) 그대로 걸렸으면 `[exact match]`, Kiwi 검색어 확장으로만 걸렸으면 `[morphological match]`. 예를 들어 `search "채권 발행"`은 `[exact match]`가 나오는지 확인해보면 된다 — 평범한 검색어는 확장 기능이 있어도 동작이 그대로여야 한다.

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
