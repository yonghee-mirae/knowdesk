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

```bash
# 1. 샘플 폴더 생성 (./samples)
cargo run -p knowdesk-core --example gen_samples

# 2. 색인 (PDF까지 검증하려면 KNOWDESK_PDFIUM_LIB_DIR=<lib 경로> 접두)
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
| `실적표.xlsx` | XLSX 추출 |
| `이사회결의.docx` | DOCX 추출 |
| `발표자료.pptx` | PPTX 추출 (슬라이드 2개, 순서 확인용) |
| `검토의견.pdf` | PDF 추출 — 한글 CID 폰트가 실제로 임베딩된 PDF (LibreOffice headless로 생성) |
| `보관용.zip` | 압축 파일 → SKIP |
| `~$규정.txt` | 임시 파일 → SKIP |
| `손상.pdf` | 손상된 PDF → META(PARSE_FAIL) |

기대 결과: `KNOWDESK_PDFIUM_LIB_DIR` 미지정 시 10건 중 6건 FULL, 2건 META(`검토의견.pdf`+`손상.pdf`, 둘 다 PARSE_FAIL), 2건 SKIP. 지정 시 `검토의견.pdf`도 FULL로 올라가 7건 FULL, 1건 META(`손상.pdf`만), 2건 SKIP.

### CLI 서브커맨드

| 명령 | 설명 |
|---|---|
| `index <경로>` | 폴더를 스캔해 색인 |
| `search <검색어> [--mode filename\|content] [--limit N]` | 검색 (기본: content 모드) |
| `stats` | 계층별(FULL/META/SKIP) 색인 건수 |
| `bench` | 벤치마크 (Phase B5에서 구현 예정, 현재 스텁) |

검색 필터는 `docs/05_Search_Language_v1.md` 문법을 그대로 따른다: `ext:pdf`, `path:리서치`, `tier:full`, `drm:true`, `modified>2026-01-01` 등을 검색어에 함께 넣으면 된다.

`--db` 옵션 없이 실행하면 현재 디렉터리에 `knowdesk.db`가 생성되므로, 테스트할 땐 `--db` 경로를 지정해 격리하는 걸 권장한다.
