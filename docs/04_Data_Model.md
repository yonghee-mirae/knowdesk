# Data Model

v1.1 개정 — 상세 근거는 `11_Implementation_Plan.md` 참조.

기존 스키마는 `documents` 테이블이 `path`/`filename`을 직접 보유하고 있었으나, 이는 PRD 7장의 "`DocumentID = SHA256(Content)`, 경로는 별도 관리"와 모순된다. 내용 해시가 PK이면 동일 내용의 사본 여러 개가 한 문서로 병합되어야 하는데, 경로를 같은 테이블에 두면 병합이 불가능하다. `documents`(내용 기준)와 `paths`(경로 기준, 1:N)로 분리한다.

---

## documents

내용(SHA256) 기준. 동일 내용의 사본은 한 행으로 병합된다.

```sql
CREATE TABLE documents
(
    document_id TEXT PRIMARY KEY,   -- SHA256(content)

    file_size INTEGER,
    text_bytes INTEGER,             -- 추출된 본문 크기 (DB 용량 추정/통계용)

    index_tier TEXT,                -- FULL | META | SKIP
    index_status TEXT,              -- 상태 머신, 하단 참조

    demotion_reason TEXT,           -- DRM | CORRUPT | ENCRYPTED | PARSE_FAIL | EMPTY_TEXT

    drm_status TEXT,
    retry_count INTEGER DEFAULT 0,
    last_attempt_at DATETIME,

    content_stored INTEGER DEFAULT 1,  -- 1=원문 저장, 0=압축/미저장 (저장 계층 전환용 플래그)

    indexed_at DATETIME
);
```

---

## paths

경로 기준. 파일 이동·이름 변경·동일 내용 사본 추적을 담당한다 (1:N, `documents` 참조).

```sql
CREATE TABLE paths
(
    path TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(document_id),

    filename TEXT NOT NULL,
    extension TEXT NOT NULL,

    modified_at DATETIME,
    seen_at DATETIME                -- 마지막으로 스캔/감시에서 확인된 시각 (오프라인 드라이브 판별용)
);
```

---

# FTS Tables

## filename_fts (제거됨)

⚠️ **변경 이력(2026-08-22):** 파일명 검색이 순수 부분 문자열("포함") 검색(SQL `LIKE`, `paths.filename` 직접 조회)으로 바뀌면서 더 이상 검색 경로에서 쓰이지 않게 되어(`05_Search_Language_v1.md` Filename Mode 참조), 테이블 자체와 색인 코드(`SearchRepository::index_filename`/`remove_filename`)를 정리했다. `db::migrate` MIGRATIONS v2가 기존 DB에서도 `DROP TABLE IF EXISTS filename_fts`로 정리한다. 아래 `CREATE VIRTUAL TABLE`은 v1이 실제로 적용했던 내용의 역사적 기록으로 `core/src/db/schema.rs`에 그대로 남아있다(마이그레이션은 과거 버전을 고쳐 쓰지 않는다).

```sql
CREATE VIRTUAL TABLE filename_fts USING fts5(filename);  -- v1에서 생성, v2에서 DROP
```

---

## content_fts

본문 검색. `body`(추출 원문 — snippet/highlight 용), `morph`(bigram, 항상 채우는 기본 토크나이저), `morph_kiwi`(Kiwi, 가능할 때만 채우는 보조 토크나이저 — 없으면 빈 문자열) 세 컬럼을 하나의 FTS 테이블로 둔다. bm25 컬럼 가중치로 세 신호를 한 번에 조합할 수 있어, 별도의 `token_fts` 테이블로 분리했을 때 필요한 조인·점수 병합을 피할 수 있다 (v1.1 대비 변경 — Phase B2에서 `morph_kiwi` 컬럼 추가, `03_Architecture.md`/`11_Implementation_Plan.md` 참조).

```sql
CREATE VIRTUAL TABLE content_fts USING fts5(body, morph, morph_kiwi);
```

검색어도 Kiwi가 있으면 형태소 분석해서 `(원문 OR morph_kiwi:(분석 형태소...))`로 확장한다 — bigram은 검색어 분석에는 쓰지 않는다(색인에만 쓰는 기본 토크나이저). 확장 없이 리터럴로 걸리면 "정확 일치", 확장을 거쳐야만 걸리면 "형태소 분석"으로 구분해 결과에 표시한다.

**`token_fts`는 별도 테이블로 두지 않는다.** (기존 v1.0 스키마 대비 변경)

---

# index_tier

FULL

META

SKIP

---

# drm_status

NON_DRM

DRM

UNKNOWN

---

# state machine

DISCOVERED

↓

PENDING

↓

EXTRACTING

↓

INDEXING

↓

INDEXED

실패 시

↓

META_INDEXED

또는

↓

FAILED

---

# 참고 — F-06(최신 버전 탐지)과의 관계

`documents`가 내용 해시 기준이므로 동일 내용 사본은 이미 한 행으로 병합되어 있고, `paths`에 경로별 `modified_at`이 있어 최신본 판별이 조회 한 번으로 가능하다. F-06 구현 시 별도의 유사도 연산 없이 이 스키마만으로 상당 부분을 충족한다.
