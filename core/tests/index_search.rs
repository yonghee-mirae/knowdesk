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
use knowdesk_core::search::service::SqliteSearchService;
use knowdesk_core::search::{SearchMode, SearchRequest, SearchService};
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
        tokenizer: &tokenizer,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 2);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService { conn: &db.conn };
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
        tokenizer: &tokenizer,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService { conn: &db.conn };
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
        tokenizer: &tokenizer,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService { conn: &db.conn };
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
        tokenizer: &tokenizer,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 1);
    assert_eq!(outcome.meta, 0);
    assert_eq!(outcome.skip, 0);

    let search = SqliteSearchService { conn: &db.conn };
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
        tokenizer: &tokenizer,
    };

    let outcome = pipeline.index_directory(dir.path()).unwrap();
    assert_eq!(outcome.full, 0);
    assert_eq!(outcome.skip, 2);
}
