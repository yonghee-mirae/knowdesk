//! Phase A 완료 기준(`docs/06_Development_Roadmap.md`)의 통합 검증:
//! 폴더를 색인하고 "채권 발행"을 검색하면 스니펫과 함께 결과가 나와야 한다.

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
    // 이 통합 검증은 실제 libpdfium 로드에 성공해야 의미가 있다 (`KNOWDESK_PDFIUM_LIB_DIR`).
    // 네이티브 라이브러리가 없는 환경(CI 등)에서도 `cargo test`가 실패하지 않도록 건너뛴다.
    if !PdfExtractor::is_available() {
        eprintln!("libpdfium을 찾을 수 없어 건너뜁니다 (KNOWDESK_PDFIUM_LIB_DIR 미설정)");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    // 한글 CID 폰트로 렌더링된 실제 PDF (LibreOffice headless로 생성, `docs/06_Development_Roadmap.md`
    // B1이 지목한 최대 리스크 구간 검증용).
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
    // 이 통합 검증은 실제 Kiwi 로드에 성공해야 의미가 있다
    // (`KNOWDESK_KIWI_LIB_PATH`/`KNOWDESK_KIWI_MODEL_DIR`). 네이티브 라이브러리가
    // 없는 환경(CI 등)에서도 `cargo test`가 실패하지 않도록 건너뛴다.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

    // "짓다"는 ㅅ 불규칙 동사라 과거형 "지었다"의 표면형에는 "짓"이라는 글자가
    // 전혀 나타나지 않는다(지/었/다). bigram은 원문 글자를 그대로 2글자씩 자르기만
    // 하므로 원문에 없는 글자로는 매칭될 수 없지만, 형태소 분석은 어간을 "짓"으로
    // 복원하므로 검색이 가능하다 — bigram 대비 Kiwi 재현율 차이를 보여주는 예시다
    // (TASK-504).
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
        "Kiwi는 지었다 → 짓 어간을 복원해 찾아야 한다"
    );
    assert_eq!(result.hits[0].filename, "공사보고서.txt");

    // 비교 기준선: 동일한 문서를 bigram으로 색인하면 "짓"은 절대 찾을 수 없다.
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
        "bigram은 원문에 없는 글자를 찾을 수 없어야 한다: {:?}",
        bigram_result.hits
    );
}

#[test]
fn expands_query_with_kiwi_to_find_dictionary_form() {
    // 검색어 쪽 형태소 분석: "짓다"(사전형)로 검색해도 활용형 "지었고"를 찾아야 한다.
    // 이건 색인이 아니라 검색어 확장(search::service::expand_with_kiwi)이 하는 일이다.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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

    // 검색어 분석 없이("짓다" 리터럴 그대로)는 찾지 못한다 — FTS5는 "짓다"를 통짜
    // 토큰 하나로 보고, 색인에는 그런 토큰이 없다.
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
        "검색어 분석 없이는 짓다로 못 찾아야 한다: {:?}",
        literal_result.hits
    );

    // 검색어를 Kiwi로 분석하면 어간 "짓"이 남아 "지었고"를 찾는다.
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

    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "공사일지.txt");
    // 리터럴 "짓다"는 원문에 없으니 형태소 분석으로만 걸린 것이어야 한다.
    assert_eq!(result.hits[0].match_kind, MatchKind::Morphological);
    // "짓다"도 "짓"도 원문에 리터럴로 없지만(ㅅ 불규칙), Kiwi가 알려주는 형태소
    // 위치로 실제 활용형 "지었고"의 원문 구간을 정확히 강조해야 한다(뒤에 붙는
    // 쉼표는 "고"와 같은 어절로 묶여 강조 범위에 함께 포함된다).
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>지었고,<<"),
        "활용형 원문 구간이 강조되지 않음: {snippet:?}"
    );
}

#[test]
fn kiwi_query_expansion_keeps_exact_tag_for_plain_noun_search() {
    // "채권 발행"처럼 평범한 명사 검색어는 검색어 확장을 켜도 리터럴 그대로 걸려야
    // 하고, 그건 "정확 일치"로 표시돼야 한다 — 확장 기능이 기존 검색을 깨면 안 된다.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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
    // "위원회에서"처럼 명사에 조사가 공백 없이 붙은 형태는, 색인 시점에 문맥이
    // 있어 Kiwi가 "위원회"를 정확히 하나의 명사로 분리해낸다(`kiwi-cli`로 직접
    // 확인). 검색어 "위원회"도 표준어라 분석 결과가 리터럴과 같으므로 확장이
    // 일어나지 않고, 그대로 "정확 일치"로 걸린다.
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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

    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "결의록.txt");
    assert_eq!(result.hits[0].match_kind, MatchKind::Exact);
}

#[test]
fn kiwi_query_expansion_still_finds_misanalyzed_compound_via_or_safety_net() {
    // 반대 사례: "이사회"는 문맥이 있어도 Kiwi가 "이(관형사)+사회"로 잘못 쪼갤 수
    // 있다("이 사회"라는 훨씬 흔한 구문과 형태가 같아서 — `kiwi-cli`로 직접 확인).
    // 색인 시점의 "이사회에서"도 검색어 "이사회"도 똑같이 잘못 쪼개지지만, 원래
    // 검색어를 교체가 아니라 OR로 남겨두는 안전망 설계 덕에 (양쪽이 공유하는
    // "사회" 조각을 통해) 문서를 찾긴 한다 — 다만 리터럴 "이사회"는 어디에도
    // 없으므로 "형태소 분석" 태그가 붙는 게 맞다(교체 방식이었다면 이 사례에서
    // 아예 못 찾았을 것이다 — "이 사회"로 쪼개진 뒤 원래 뜻과 무관해지므로).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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

    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);
    assert_eq!(result.hits[0].filename, "결의록.txt");
    assert_eq!(result.hits[0].match_kind, MatchKind::Morphological);
}

#[test]
fn highlights_literal_match_in_original_text_when_body_column_has_none() {
    // "레이아웃과"처럼 명사에 조사가 공백 없이 붙어 있으면 body 컬럼엔 그 토큰이
    // 없다(FTS5 기본 토크나이저는 "레이아웃과"를 통짜 토큰으로 본다). Kiwi가 색인
    // 시점에 "레이아웃"을 조사와 분리해 morph_kiwi에만 넣어두므로, "레이아웃"으로
    // 검색하면 body엔 강조할 게 없어 스니펫에 아무 표시도 안 뜨는 문제가 실제로
    // 있었다. body에 강조가 없으면 저장된 원문에서 검색어를 직접 찾아 강조해야
    // 한다 — FTS5 컬럼 자동 선택으로는 토큰만 나열된 morph_kiwi 텍스트가 나와서
    // (예: "발행 시 이사회 승인 필요 다 달 레이아웃 표 검증") 오히려 더 혼란스럽다
    // (실제로 그렇게 나오는 것까지 확인하고 되돌림).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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

    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>레이아웃<<"),
        "스니펫에 강조 표시가 없음: {snippet:?}"
    );
    // 토큰 나열이 아니라 원문 그대로여야 한다 — 조사가 붙은 자연스러운 표현
    // "레이아웃과"가 강조 주변에 그대로 남아있는지 확인한다.
    assert!(
        snippet.contains(">>레이아웃<<과"),
        "원문이 아니라 토큰 나열이 나온 것으로 보임: {snippet:?}"
    );
}

#[test]
fn highlights_kiwi_analyzed_stem_when_typed_query_itself_is_absent() {
    // "수행함"으로 검색하면 Kiwi가 어간 "수행"으로 확장해 매칭시키지만, 원문엔
    // "수행함"이 아니라 "수행한다"만 있다 — 타이핑한 검색어 그대로는 원문에서
    // 못 찾는다. 이럴 땐 실제 매칭에 쓰인 어간("수행")으로 원문을 찾아 강조해야
    // 한다(실제로 강조 없이 문서 맨 앞부분만 나오는 문제가 있었다).
    let Some(kiwi_result) = KiwiTokenizer::from_env() else {
        eprintln!("KNOWDESK_KIWI_LIB_PATH/KNOWDESK_KIWI_MODEL_DIR 미설정, 건너뜁니다");
        return;
    };
    let kiwi = kiwi_result.expect("Kiwi 초기화 실패");

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

    assert_eq!(result.hits.len(), 1, "히트: {:?}", result.hits);
    let snippet = result.hits[0].snippet.as_deref().unwrap_or_default();
    assert!(
        snippet.contains(">>수행<<한다"),
        "어간이 원문에서 강조되지 않음: {snippet:?}"
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
