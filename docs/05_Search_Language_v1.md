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

---

# Filters

## Extension

```text
ext:pdf
```

---

## Path

```text
path:리서치
```

---

## Tier

```text
tier:full
```

```text
tier:meta
```

---

## DRM

```text
drm:true
```

---

## Modified

```text
modified>2026-01-01
```

```text
modified<2026-08-01
```

---

# Search Modes

## Filename Mode

filename_fts만 대상

---

## Content Mode

content_fts (body, morph)

bm25 컬럼 가중치로 body·morph 두 신호를 조합한다. 별도의 token_fts 테이블은 두지 않는다 (`04_Data_Model.md` 참조).
