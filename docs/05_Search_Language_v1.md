# Search Language v1

v1.1 개정 — `token_fts` 참조 제거(`04_Data_Model.md`에서 `content_fts(body, morph)` 단일 테이블로 통합됨), PRD 필터 목록과의 불일치 정리.

## Keyword

```text
채권
```

---

## Phrase

```text
"채권 발행"
```

---

## AND

```text
채권 AND 발행
```

---

## OR

```text
채권 OR 회사채
```

---

## NOT

```text
채권 NOT 국채
```

---

## Prefix

```text
발행*
```

뒤쪽에 붙는 접두(prefix) 형태만 지원한다. `*발행`(앞쪽)이나 `*발행*`(양쪽)은 와일드카드로 동작하지 않는다 — 아래 Limitations 참조.

---

## Grouping

```text
(채권 OR 회사채) AND 발행
```

괄호로 우선순위를 명시할 수 있다. 괄호 없이 쓰면 AND가 OR보다 먼저 묶인다 — `채권 OR 회사채 AND 발행`은 `채권 OR (회사채 AND 발행)`과 같다.

⚠️ **수정 이력(2026-08-22):** 한때는 특수문자 이스케이프 로직(`sanitize_term`, `core/src/search/parser.rs`)이 `(`/`)`를 통째로 리터럴 문구로 감싸버려서 괄호가 조용히 무시되는 버그가 있었다 — 에러 없이 그룹핑 안 한 것과 동일한(틀린) 결과가 나와서 알아채기 어려웠다. 수정 완료. `core/tests/index_search.rs`의 `grouping_parens_change_the_result_from_the_ungrouped_query`가 그룹 있음/없음이 실제로 다른 결과를 내는지 검증한다.

---

## Literal AND / OR / NOT

`AND`/`OR`/`NOT`은 그대로 쓰면 연산자로 해석된다. 이 단어 자체를 검색어로 찾고 싶다면 Phrase와 동일하게 따옴표로 감싼다:

```text
"AND"
```

연산자 해석을 건너뛰고 리터럴 문구로 취급된다. 실제 문서("Party A AND Party B agree...")로 검증함: `Party AND Party`는 Party 두 개를 찾고(AND는 연산자로 소비됨), `"AND"`는 AND라는 단어 자체를 찾는다.

---

# Filters

필터 접두어는 한 글자다(`x:`/`p:`/`m>`/`m<`/`m=`) — 사용자가 직접 타이핑해야 하므로 길면 입력이 번거롭다. ⚠️ **변경 이력(2026-08-22):** 원래 `ext:`/`path:`/`modified` 세 글자 이상이었던 것을 줄였고, `tier:`/`drm:` 필터는 아예 없앴다(진단용으로도 잘 안 쓰여 유지 비용만 있다고 판단 — `KnowDesk_추가검토사항.md` 참조). `documents.index_tier`/`drm_status` 컬럼과 결과 화면의 배지 표시는 그대로 남아 있다 — 검색어로 타이핑해서 걸러내는 기능만 없앤 것이다.

키워드 없이 필터만 써도 된다(예: `x:pdf`만 입력). ⚠️ **수정 이력(2026-08-22):** 한때는 필터만 쓰고 키워드가 없으면 `match_expr`이 빈 문자열이 되어 FTS5가 `MATCH ''`를 구문 오류로 거부했다("fts5: syntax error near ''") — `x:pdf` 단독 검색(당시는 `ext:pdf`)이 전부 에러였다. 키워드가 없으면 애초에 관련도 랭킹(bm25)도 의미가 없으므로, 이 경우 FTS5 가상 테이블을 아예 거치지 않고 `paths`/`documents`를 직접 조회해 최신 수정일 순으로 나열하도록 고쳤다(`core/src/db/search_repo.rs`의 `search_filename_filters_only`/`search_content_filters_only`). `core/src/db/search_repo.rs`의 `search_filename_with_only_filters_and_no_keyword_does_not_crash`/`search_content_with_only_filters_dedupes_by_document`로 검증.

## Extension

```text
x:pdf
```

---

## Path

```text
p:리서치
```

---

## Modified

```text
m>2026-01-01
```

```text
m<2026-08-01
```

```text
m=2026-08-10
```

`m=`은 특정 날짜 하루에 수정된 문서만 찾는다(추가됨, 2026-08-22). `paths.modified_at`는 시각까지 포함한 전체 타임스탬프라 문자열을 그대로 비교(`=`)하면 절대 일치하지 않는다 — SQL에서 양쪽을 `date(...)`로 감싸 날짜 부분만 비교한다(`core/src/db/search_repo.rs`).

---

# Search Modes

## Filename Mode

filename_fts만 대상

---

## Content Mode

content_fts (body, morph)

bm25 컬럼 가중치로 body·morph 두 신호를 조합한다. 별도의 token_fts 테이블은 두지 않는다 (`04_Data_Model.md` 참조).

---

# Limitations

## 구두점은 검색어로 쓸 수 없음 (확인됨, 2026-08-22)

`(`, `)`, `.`, `,` 같은 구두점은 색인 시점에 FTS5 기본 토크나이저(unicode61)가 단어 구분자로 취급해 애초에 토큰으로 저장하지 않는다. 검색어 쪽에서 아무리 잘 감싸도(`"("`처럼 따옴표로 묶어도) 색인 자체에 없는 토큰을 찾을 수는 없다.

실제 문서("...the terms (see appendix).")로 검증함: 문서에 괄호가 분명히 있어도 `"("` 검색은 결과 없음. Grouping 버그(위 참조)와 근본 원인은 같지만(FTS5가 구두점을 토큰 취급 안 함), 이쪽은 파서를 고쳐서 해결할 수 있는 문제가 아니라 이 검색엔진 구성의 구조적 한계다. 현재로선 해결 계획 없음.

## 앞쪽·양쪽 와일드카드는 지원하지 않음 (확인됨, 2026-08-22)

FTS5는 뒤쪽 접두(prefix) 와일드카드(`발행*`)만 지원한다. 역색인이 접두사 스캔에만 최적화돼 있어서, `*발행`(앞쪽)이나 `*발행*`(양쪽)처럼 임의 위치를 찾으려면 전문(全文) 스캔이 필요한데 이는 정규표현식을 지원하지 않는 것과 같은 이유(`01_KnowDesk_PRD.md` 5장)다.

`*`가 앞에 붙은 검색어는 `sanitize_term`이 안전하지 않은 문자로 판단해 리터럴 구문으로 감싸는데, FTS5가 구문 내용도 구두점을 제거하며 토큰화하므로 `*`가 조용히 사라지고 **와일드카드 없는 평범한 키워드 검색으로 저하된다** — 에러도 없고 결과도 그럴듯하게 나와서(아래 참조) 알아채기 어렵다.

실제 문서("채권 재발행 절차...")로 검증함:

| 검색어 | 매치 | 하이라이트 |
|---|---|---|
| `*발행` | 됨 (저하된 키워드 검색으로) | 안 됨 |
| `*발행*` | 됨 (동일) | 안 됨 |
| `발행` (와일드카드 없이) | 됨 | **됨** (`재>>발행<<`) |

`발행`만으로도 이미 "재발행" 안에서 찾아지고 하이라이트도 정확한 이유는 별도의 와일드카드 로직이 아니라, 이 프로젝트가 한국어 대응을 위해 **bigram(2글자 슬라이딩 윈도우) 토크나이저를 기본으로 항상 색인**하기 때문이다(`core/src/nlp/bigram.rs`) — "재발행"을 색인하면 "재발"/"발행" 두 bigram이 모두 만들어진다. 즉 2글자 단위로 단어 중간에 묻힌 조각을 찾는 실질적 필요는 와일드카드 없이 이미 충족된다. 그보다 긴 임의 위치 부분 문자열은 지금 구조로 안 되고, 하려면 FTS5 `trigram` 토크나이저로 교체하는 등 더 큰 설계 변경이 필요하다 — 현재로선 계획 없음.
