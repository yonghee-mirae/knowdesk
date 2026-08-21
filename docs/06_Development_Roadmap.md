# Development Roadmap

v1.1 개정 — M0~M15 선형 순서 대신, **동작하는 end-to-end 파이프라인을 최대한 빨리 세우는 순서**로 재배열했다 (`11_Implementation_Plan.md` 참조). 추출 포맷을 전부 만든 뒤 검색을 붙이면 마지막까지 아무것도 검증할 수 없기 때문이다. Windows 환경 없이 Linux에서 검증 가능한 항목을 Phase A~C에 최대한 몰아넣고, Windows 종속 항목만 Phase D로 분리했다.

---

# Phase A — Walking Skeleton (Linux 전량 검증 가능, 완료)

## A1 Foundation

- Cargo Workspace 구성 (core / cli / src-tauri / frontend)
- Logging
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

순서: XLSX → DOCX/PPTX → PDF (PDF 한글 CID 폰트 검증이 가장 리스크가 큰 구간이므로 마지막에 배치)

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

## B3 동의어 사전

- Synonym Engine (질의 시점 확장)

## B4 File Monitoring

- FileSystemWatcher (notify)
- EventQueue
- Debounce (Office 저장 시 임시파일 폭풍 대응 — 필수)

## B5 Benchmark

- Benchmark Harness (`cli bench`) — 색인 처리량, 검색 P95, DB 실측 크기

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

- Settings Window
- Statistics
- Logs

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
