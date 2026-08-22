//! Integration verification for Phase A's completion criteria (`docs/06_Development_Roadmap.md`):
//! indexing a folder and searching "채권 발행" should return results with a snippet.

use knowdesk_core::config::Config;
use knowdesk_core::db::Db;
use knowdesk_core::extract::ooxml::DocxExtractor;
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::xlsx::XlsxExtractor;
use knowdesk_core::extract::ContentExtractor;
use knowdesk_core::index::pipeline::IndexPipeline;
use knowdesk_core::nlp::bigram::BigramTokenizer;
use knowdesk_core::nlp::kiwi::KiwiTokenizer;
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{MatchKind, SearchMode, SearchRequest, SearchService};
use std::io::Write;
use zip::write::SimpleFileOptions;

#[test]
fn indexes_sample_folder_and_finds_snippet() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("규정.txt"),
        "본 문서는 채권 발행 절차를 규정한다. 채권 발행 시 이사회 승인이 필요하다.",
    )
    .unwrap();
    std::fs::write(dir.path().join("무관.txt"), "회의록 요약: 다음 분기 예산안").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let tokenizer = BigramTokenizer;

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 2);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 발행".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    let hit = &result.hits[0];
    assert_eq!(hit.filename, "규정.txt");
    assert!(hit.snippet.as_deref().unwrap().contains(">>"));
    assert_eq!(hit.extension, "txt");
    assert_eq!(hit.index_tier, "FULL");
    assert!(hit.modified_at.is_some());
}

#[test]
fn grouping_parens_change_the_result_from_the_ungrouped_query() {
    // Real bug, confirmed against the actual FTS5 index: `sanitize_term` used to quote
    // `(건물`/`채권)` as literal phrases, and FTS5 tokenizes phrase content the same way
    // it tokenizes indexed text — stripping the `(`/`)` as punctuation — so the phrase
    // silently degraded to matching bare `건물`/`채권` and the grouping just vanished,
    // with no error. `(건물 OR 채권) AND 결의` and `건물 OR 채권 AND 결의` returned
    // identical results. This test pins the fix: with real grouping, `공사.txt` (which
    // has "건물" but no "결의") must be excluded by the parenthesized query but not by
    // the ungrouped one.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("공사.txt"), "그는 새 건물을 지었다.").unwrap();
    std::fs::write(
        dir.path().join("이사회.txt"),
        "이사회 결의를 통해 채권 발행을 승인한다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let bigram = BigramTokenizer;
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: None,
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let search_for = |query: &str| {
        search
            .search(&SearchRequest {
                query: query.to_string(),
                mode: SearchMode::Content,
                limit: 10,
            })
            .unwrap()
            .hits
    };

    // Ungrouped: AND binds tighter than OR by default, so this is
    // `건물 OR (채권 AND 결의)` — both documents satisfy one side or the other.
    let ungrouped = search_for("건물 OR 채권 AND 결의");
    assert_eq!(ungrouped.len(), 2, "hits: {:?}", ungrouped);

    // Grouped: `(건물 OR 채권) AND 결의` requires "결의" no matter what, which
    // "공사.txt" doesn't have — only "이사회.txt" should match.
    let grouped = search_for("(건물 OR 채권) AND 결의");
    assert_eq!(grouped.len(), 1, "hits: {:?}", grouped);
    assert_eq!(grouped[0].filename, "이사회.txt");
}

#[test]
fn filename_search_populates_metadata_fields() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("규정.txt"), "본문은 검색 대상이 아니다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let tokenizer = BigramTokenizer;
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "규정".to_string(),
            mode: SearchMode::Filename,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    let hit = &result.hits[0];
    assert_eq!(hit.extension, "txt");
    assert_eq!(hit.index_tier, "FULL");
    assert!(hit.modified_at.is_some());
}

#[test]
fn indexes_xlsx_file_and_finds_snippet() {
    let dir = tempfile::tempdir().unwrap();
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "채권").unwrap();
    sheet.write_string(0, 1, "발행").unwrap();
    sheet.write_string(1, 0, "절차").unwrap();
    workbook.save(dir.path().join("규정.xlsx")).unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> =
        vec![Box::new(TxtExtractor), Box::new(XlsxExtractor)];
    let tokenizer = BigramTokenizer;

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 발행".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].filename, "규정.xlsx");
}

#[test]
fn indexes_docx_file_and_finds_snippet() {
    let dir = tempfile::tempdir().unwrap();
    let file = std::fs::File::create(dir.path().join("규정.docx")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("word/document.xml", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        r#"<w:document xmlns:w="http://x"><w:body>
            <w:p><w:r><w:t>채권</w:t></w:r><w:r><w:t> 발행</w:t></w:r></w:p>
            <w:p><w:r><w:t>절차</w:t></w:r></w:p>
        </w:body></w:document>"#
            .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> =
        vec![Box::new(TxtExtractor), Box::new(DocxExtractor)];
    let tokenizer = BigramTokenizer;

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 발행".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].filename, "규정.docx");
}

#[test]
fn indexes_pdf_file_and_finds_snippet() {
    // This integration test is only meaningful if native libpdfium actually loads
    // (`KNOWDESK_PDFIUM_LIB_DIR`). Skipped so `cargo test` doesn't fail in environments
    // without the native library (e.g. CI).
    if !PdfExtractor::is_available() {
        eprintln!("libpdfium not found, skipping (KNOWDESK_PDFIUM_LIB_DIR not set)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    // Real PDF rendered with a Korean CID font (generated via LibreOffice headless, for
    // verifying the highest-risk area flagged for B1 in `docs/06_Development_Roadmap.md`).
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/korean.pdf"),
        dir.path().join("규정.pdf"),
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> =
        vec![Box::new(TxtExtractor), Box::new(PdfExtractor)];
    let tokenizer = BigramTokenizer;

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 발행".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].filename, "규정.pdf");
}

#[test]
fn finds_irregular_verb_stem_only_with_kiwi() {
    // This integration test is only meaningful if Kiwi actually loads
    // (`KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`). Skipped so `cargo test` doesn't
    // fail in environments without the native library (e.g. CI).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    // "짓다" is a ㅅ-irregular verb, so its past-tense surface form "지었다" never actually
    // contains the character "짓" (지/었/다). Since bigram just slices the original text two
    // characters at a time, it can't match a character absent from the original text, but
    // morphological analysis recovers the stem as "짓", making it searchable — an example
    // showing the recall gap between bigram and Kiwi (TASK-504).
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("공사보고서.txt"), "그는 새 건물을 지었다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &BigramTokenizer,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "짓".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert_eq!(
        result.hits.len(),
        1,
        "Kiwi should recover the stem 지었다 → 짓 and find it"
    );
    assert_eq!(result.hits[0].filename, "공사보고서.txt");

    // Baseline for comparison: indexing the same document with bigram should never find "짓".
    let bigram_db = Db::open_in_memory().unwrap();
    let bigram = BigramTokenizer;
    let bigram_pipeline = IndexPipeline {
        conn: &bigram_db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: None,
    };
    bigram_pipeline.index_directory(dir.path()).unwrap();

    let bigram_search = SqliteSearchService {
        conn: &bigram_db.conn,
        kiwi: None,
    };
    let bigram_result = bigram_search
        .search(&SearchRequest {
            query: "짓".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        bigram_result.hits.is_empty(),
        "bigram must not be able to find a character absent from the original text: {:?}",
        bigram_result.hits
    );
}

#[test]
fn kiwi_marks_irregular_stem_match_as_morphological_not_exact() {
    // Bug report (2026-08-22): searching the already-literal stem "짓" against "지었다"
    // (ㅅ-irregular past tense of 짓다) was tagged "exact match" — wrong. "짓" never
    // appears in "지었다" (지/었/다); it's only findable at all because Kiwi's
    // morphological analysis put the stem into `morph_kiwi` at index time. Match-kind was
    // deciding "exact" purely from "the query wasn't expanded", but an unexpanded query
    // (searching a term that already equals its own stem) can still only be found via
    // morph_kiwi, never literally in the source — this is exactly that case, and unlike
    // `finds_irregular_verb_stem_only_with_kiwi` above, keeps Kiwi active at search time
    // too, since confirming "morphological, not exact" requires `Tokenizer::locate`.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("공사보고서.txt"), "그는 새 건물을 지었다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &BigramTokenizer,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "짓".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    assert_eq!(result.hits[0].match_kind, MatchKind::Morphological);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>지었다.<<"),
        "should highlight the original-text span the stem was recovered from: {snippet:?}"
    );
}

#[test]
fn expands_query_with_kiwi_to_find_dictionary_form() {
    // Morphological analysis on the query side: searching with the dictionary form "짓다"
    // should still find the inflected form "지었고". This is done by query expansion
    // (search::service::expand_with_kiwi), not by indexing.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("공사일지.txt"),
        "그는 지난달 새 창고를 지었고, 다음 달엔 담장을 세울 예정이다.",
    )
    .unwrap();
    std::fs::write(dir.path().join("무관.txt"), "회의록 요약: 다음 분기 예산안").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    // Without query analysis (literal "짓다" as-is), it should not be found — FTS5 treats
    // "짓다" as a single opaque token, and the index has no such token.
    let no_expansion = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let literal_result = no_expansion
        .search(&SearchRequest {
            query: "짓다".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();
    assert!(
        literal_result.hits.is_empty(),
        "without query analysis, 짓다 should not be found: {:?}",
        literal_result.hits
    );

    // Analyzing the query with Kiwi leaves the stem "짓", which finds "지었고".
    let with_expansion = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = with_expansion
        .search(&SearchRequest {
            query: "짓다".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "공사일지.txt");
    // The literal "짓다" is absent from the original text, so it must have matched only
    // via morphological analysis.
    assert_eq!(result.hits[0].match_kind, MatchKind::Morphological);
    // Neither "짓다" nor "짓" is literally present in the original text (ㅅ-irregular), but
    // using the morpheme position Kiwi reports, the exact original-text span of the actual
    // inflected form "지었고" must be highlighted (the trailing comma is grouped into the
    // same word as "고" and included in the highlight range).
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>지었고,<<"),
        "the original-text span of the inflected form was not highlighted: {snippet:?}"
    );
}

#[test]
fn kiwi_query_expansion_keeps_exact_tag_for_plain_noun_search() {
    // For a plain noun query like "채권 발행", even with query expansion enabled it should
    // still match literally, and be tagged "exact match" — the expansion feature must not
    // break existing search behavior.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("규정.txt"),
        "본 문서는 채권 발행 절차를 규정한다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 발행".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].match_kind, MatchKind::Exact);
}

#[test]
fn kiwi_query_expansion_finds_compound_noun_attached_to_particle() {
    // A form like "위원회에서" where a particle is attached to a noun with no space: since
    // there's context at index time, Kiwi correctly separates out "위원회" as a single noun
    // (confirmed directly via `kiwi-cli`). Since the query "위원회" is also standard
    // vocabulary, its analysis result equals the literal, so no expansion happens, and it
    // matches directly as an "exact match".
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("결의록.txt"),
        "위원회에서 채권 발행을 승인했다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "위원회".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "결의록.txt");
    assert_eq!(result.hits[0].match_kind, MatchKind::Exact);
}

#[test]
fn kiwi_query_expansion_still_finds_misanalyzed_compound_via_or_safety_net() {
    // Even with context, Kiwi can incorrectly split "이사회" into "이(determiner)+사회"
    // (because it shares its form with the much more common phrase "이 사회" — confirmed
    // directly via `kiwi-cli`). Both the indexed "이사회에서" and the query "이사회" get
    // split the same way incorrectly, but thanks to the safety-net design that keeps the
    // original query as an OR rather than replacing it, the document is still found (via
    // the shared "사회" fragment) — with a replacement approach, this case would have
    // found nothing at all (once split into "이 사회" it becomes unrelated to the
    // original meaning).
    //
    // Even though the FTS match itself came through that misanalyzed "사회" fragment,
    // this is still tagged "exact match": the literal characters "이사회" are right there
    // in the source text ("이사회에서"), just not as their own separate FTS token (no
    // space before "에서") — a bug fix (2026-08-22) made match-kind track the same
    // literal-vs-Kiwi-located-span check the highlight already used, instead of whether
    // the *query* needed Kiwi expansion at all. Before that fix this asserted
    // `Morphological` on the reasoning that "the literal 이사회 appears nowhere" — which
    // was simply wrong: it appears right there as a substring, and the snippet assertion
    // below already proved it by highlighting exactly that span.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("결의록.txt"),
        "이사회에서 채권 발행을 승인했다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "이사회".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "결의록.txt");
    assert_eq!(result.hits[0].match_kind, MatchKind::Exact);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>이사회<<"),
        "the literal substring should be highlighted even though FTS5 found it via the \
         misanalyzed 사회 fragment: {snippet:?}"
    );
}

#[test]
fn highlights_literal_match_in_original_text_when_body_column_has_none() {
    // When a particle is attached to a noun with no space, like "레이아웃과", the body column
    // doesn't contain that token (FTS5's default tokenizer treats "레이아웃과" as one opaque
    // token). Since Kiwi separates "레이아웃" from the particle at index time and puts it only
    // in morph_kiwi, searching for "레이아웃" used to have nothing to highlight in body, and a
    // real bug caused the snippet to show no highlight at all. When body has no highlight, the
    // query must be searched for directly in the stored original text and highlighted there —
    // FTS5's automatic column selection would instead surface the morph_kiwi text, which is
    // just a list of tokens (e.g. "발행 시 이사회 승인 필요 다 달 레이아웃 표 검증") and is
    // actually more confusing (confirmed this happens and reverted it).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("검토의견.txt"),
        "다단 레이아웃과 표 검증은 별도로 수행한다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "레이아웃".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>레이아웃<<"),
        "no highlight in the snippet: {snippet:?}"
    );
    // It must be the actual original text, not a list of tokens — verify that the natural
    // expression "레이아웃과" (with the attached particle) still surrounds the highlight.
    assert!(
        snippet.contains(">>레이아웃<<과"),
        "it looks like a list of tokens came out instead of the original text: {snippet:?}"
    );
}

#[test]
fn highlights_kiwi_analyzed_stem_when_typed_query_itself_is_absent() {
    // Searching for "수행함" makes Kiwi expand it to match the stem "수행", but the original
    // text only has "수행한다", not "수행함" — the typed query itself can't be found in the
    // original text. In this case, the original text must be located and highlighted using
    // the stem actually used for matching ("수행") (a real bug caused the document to show
    // with no highlight, just its beginning).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR not set, skipping");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi initialization failed");

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("검토의견.txt"),
        "다단 레이아웃과 표 검증은 별도로 수행한다.",
    )
    .unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let bigram = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &bigram,
        kiwi: Some(&kiwi),
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: Some(&kiwi),
    };
    let result = search
        .search(&SearchRequest {
            query: "수행함".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>수행<<한다"),
        "the stem was not highlighted in the original text: {snippet:?}"
    );
}

#[test]
fn widens_highlight_to_include_trailing_symbol_not_part_of_any_fts5_token() {
    // FTS5's default tokenizer treats "%" as a separator and excludes it from tokens. So a
    // real bug caused a search for "3.2%" to produce a snippet like ">>3.2<<%", where the
    // highlight was shorter than the query — when the text right after the highlight
    // continues the rest of the query, the highlight must be widened.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("보고서.txt"), "GDP 성장률은 3.2%였다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let tokenizer = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "3.2%".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>3.2%<<"),
        "% was not included in the highlight: {snippet:?}"
    );
}

#[test]
fn highlights_every_or_term_not_just_the_first_ones_fts5_native_highlight_covers() {
    // Bug report: searching "채권 OR 규정" only highlighted "채권", even though "규정" is
    // right there in the same snippet excerpt (inside "규정한다"). FTS5's own snippet()
    // highlighted "채권" (a literal body token) but not "규정" (only reachable via the
    // bigram column, since "규정한다" is one opaque token to FTS5's default tokenizer) —
    // and since *some* highlight was present, the single-needle fallback rebuild never
    // ran, silently dropping the second term's highlight entirely.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("규정.txt"), "채권 발행 절차를 규정한다.").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let tokenizer = BigramTokenizer;
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };
    pipeline.index_directory(dir.path()).unwrap();

    let search = SqliteSearchService {
        conn: &db.conn,
        kiwi: None,
    };
    let result = search
        .search(&SearchRequest {
            query: "채권 OR 규정".to_string(),
            mode: SearchMode::Content,
            limit: 10,
        })
        .unwrap();

    assert_eq!(result.hits.len(), 1, "hits: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>채권<<"),
        "채권 should be highlighted: {snippet:?}"
    );
    assert!(
        snippet.contains(">>규정<<"),
        "규정 should also be highlighted, not just the first OR term: {snippet:?}"
    );
}

#[test]
fn skips_oversized_and_excluded_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("archive.zip"), b"not a real zip").unwrap();
    std::fs::write(dir.path().join("~$temp.txt"), b"temp").unwrap();

    let db = Db::open_in_memory().unwrap();
    let config = Config::default();
    let extractors: Vec<Box<dyn ContentExtractor>> = vec![Box::new(TxtExtractor)];
    let tokenizer = BigramTokenizer;

    let pipeline = IndexPipeline {
        conn: &db.conn,
        config: &config,
        extractors: &extractors,
        bigram: &tokenizer,
        kiwi: None,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 0);
    assert_eq!(outcome.skip, 2);
}
