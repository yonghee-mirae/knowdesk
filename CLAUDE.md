# CLAUDE.md

# KnowDesk

Your Knowledge, On Your Desk.

---

# Mandatory Reading Order

Before implementing any feature, read below files in `@docs` directory:

1. 01_KnowDesk_PRD.md
2. 11_Implementation_Plan.md
3. 03_Architecture.md
4. 04_Data_Model.md
5. 05_Search_Language_v1.md
6. 06_Development_Roadmap.md
7. 07_Coding_Agent_Backlog.md
8. 08_API_Contracts.md
9. 10_DRM_Integration.md
10. 02_Known_Issues.md
11. 12_UI_Spec.md
12. KnowDesk_추가검토사항.md
13. 13_CLI_Tool.md

PRD is the source of truth for product requirements.

If requirements conflict, follow:

```text
01_KnowDesk_PRD
  ↓
11_Implementation_Plan   (기술 스택·아키텍처 결정 — PRD의 C#/.NET 표기 및 단일 파일 배포·HMAC 조항을 대체한다)
  ↓
Everything Else
```

---

# Technology Stack

## Language & Runtime

- Rust (workspace)
- Tauri 2 (packaging / OS 통합)

## Core

- `rusqlite` (bundled, FTS5)
- `kiwi-rs` (형태소 분석, LGPL-2.1+, 동적 라이브러리+모델 동봉)
- `pdfium-render` (PDF)
- `calamine` (XLSX)
- `zip` + `quick-xml` (DOCX/PPTX)
- `encoding_rs` + `chardetng` (TXT 인코딩 감지, CP949/EUC-KR 대응)
- `notify` (파일 감시)

## Frontend

- TypeScript
- Web Components
- Vite

Forbidden:

- React
- Vue
- Angular
- Svelte

---

# Architecture Rules

## Crate Structure

```text
knowdesk/
├── core/        # 순수 비즈니스 로직. Tauri를 절대 참조하지 않는다.
├── cli/         # 헤드리스 검증 하니스 (index / search / stats / bench) + kdfind (사전 색인 없는 단독 배포용 검색 도구, `docs/13_CLI_Tool.md`)
├── src-tauri/   # 트레이·전역단축키·IPC만
└── frontend/    # TS + Web Components + Vite
```

`core`는 Tauri API를 알아서는 안 된다. 모든 OS 통합은 `src-tauri`로 격리한다.

## Key Traits

- `ContentExtractor` — 문서 본문 추출 추상화 (Non-DRM / DRM API / Trusted Process)
- `Tokenizer` — 형태소 분석기 교체 가능하게 (bigram → Kiwi)
- `DocumentStore` — 원문 저장 방식 추상화 (원문 저장 → 압축 하이브리드 전환 가능)

세부 시그니처는 `docs/08_API_Contracts.md` 참조.

---

# Decision Log (요약)

문서 작성 당시 C#/.NET을 전제로 했던 부분과, PRD의 단일 실행 파일·HMAC 토큰 조항은 아래와 같이 개정되었다. 상세 근거는 `docs/11_Implementation_Plan.md` 참조.

| 항목 | 기존 | 개정 |
|---|---|---|
| 언어/런타임 | C# / .NET | Rust + Tauri 2 |
| 배포 형태 | 단일 실행 파일 | 인스톨러 (Kiwi·PDFium 네이티브 동봉) |
| 토큰 저장 | HMAC 비가역 | 평문 저장 (원문도 저장 허용) |
| 문서 식별자 | SHA256(Content), 경로 별도 관리 | 변경 없음 (그대로 유지) |
| 개발 환경 | Windows 전제 | Linux에서 헤드리스 코어 우선 검증, Windows 종속은 Phase D |

---

# Coding Style

## Rust

- 단순한 모듈 구조, 명시적 타입
- 불필요한 매크로·과도한 제네릭 추상화 지양
- `core`는 trait으로 확장점만 노출하고 구현체는 교체 가능하게 유지

## TypeScript

- `strict: true`
- 작은 모듈, 명시적 인터페이스·반환 타입
- `any` 금지

---

# Agent 작업 메모

다른 PC에서 이어서 작업할 때도 적용되도록 저장소에 함께 커밋해두는 메모. (개인 홈 디렉터리의 Claude 메모리는 PC별로 분리되어 다른 PC에서는 보이지 않는다.)

- **git 작업은 사용자가 직접 한다.** Claude는 커밋·푸시 등을 제안하거나 실행하지 않는다. 상태 확인(`git status`/`git log` 등 읽기 전용)만 필요 시 수행한다.
