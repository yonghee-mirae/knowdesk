//! Sample folder generator for manual testing.
//!
//! Usage:
//! ```text
//! cargo run -p knowdesk-core --example gen_samples [output path, default ./samples]
//! ```
//!
//! Generates normal cases together with exclusion-rule cases so the currently implemented
//! scope (Phase A + Phase B1 XLSX/DOCX/PPTX/PDF) can be manually verified in one go. The
//! output can be regenerated at any time, so it is not committed to git (see `.gitignore`).
//!
//! PDF extraction only actually works if native libpdfium is available. If the library path
//! isn't set via the `KNOWDESK_PDFIUM_LIB_DIR` environment variable, `검토의견.pdf` is
//! downgraded to META (this is not an error — it's this project's normal downgrade policy).

use std::fs;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./samples".to_string());
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("failed to create output folder");

    write_txt_utf8(out_dir);
    write_txt_euckr(out_dir);
    write_txt_unrelated(out_dir);
    write_txt_irregular_verb(out_dir);
    write_xlsx(out_dir);
    write_docx(out_dir);
    write_pptx(out_dir);
    write_pdf(out_dir);
    write_skip_cases(out_dir);

    println!("Sample generation complete: {}", out_dir.display());
    println!();
    println!("How to verify:");
    println!(
        "  cargo run -p knowdesk-cli -- --db ./samples.db index {}",
        out_dir.display()
    );
    println!("  cargo run -p knowdesk-cli -- --db ./samples.db search \"채권 발행\"");
    println!(
        "  cargo run -p knowdesk-cli -- --db ./samples.db search \"짓다\"  # With Kiwi enabled, the query is also morphologically analyzed — finds inflected forms via the dictionary form"
    );
}

fn write_txt_utf8(dir: &Path) {
    fs::write(
        dir.join("규정.txt"),
        "본 문서는 채권 발행 절차를 규정한다.\n채권 발행 시 이사회 승인이 필요하다.\n",
    )
    .unwrap();
}

fn write_txt_euckr(dir: &Path) {
    // For verifying automatic encoding detection (encoding_rs + chardetng) — handles documents saved as CP949/EUC-KR.
    let (bytes, _, had_errors) =
        encoding_rs::EUC_KR.encode("회의록: 채권 발행 계획을 다음 분기로 연기한다.");
    assert!(!had_errors);
    fs::write(dir.join("회의록_EUCKR.txt"), bytes).unwrap();
}

fn write_txt_unrelated(dir: &Path) {
    fs::write(dir.join("무관.txt"), "회의록 요약: 다음 분기 예산안 검토\n").unwrap();
}

fn write_txt_irregular_verb(dir: &Path) {
    // "짓다" is a ㅅ-irregular verb, so its past-tense surface form "지었다" never actually
    // contains the character "짓". Since bigram just slices the original text two characters
    // at a time, it can never find it via "짓", but Kiwi recovers the stem and finds it.
    // It's found both by the query "짓" (the stem) and by the dictionary form "짓다" — because
    // Kiwi recovers the stem at index time (`content_fts.morph_kiwi`) and also morphologically
    // analyzes and expands the query (same case as `finds_irregular_verb_stem_only_with_kiwi`
    // and `expands_query_with_kiwi_to_find_dictionary_form` in `core/tests/index_search.rs`).
    fs::write(dir.join("공사보고서.txt"), "그는 새 건물을 지었다.\n").unwrap();
}

fn write_xlsx(dir: &Path) {
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.write_string(0, 0, "채권").unwrap();
    sheet.write_string(0, 1, "발행").unwrap();
    sheet.write_string(1, 0, "실적").unwrap();
    workbook.save(dir.join("실적표.xlsx")).unwrap();
}

fn write_docx(dir: &Path) {
    let file = fs::File::create(dir.join("이사회결의.docx")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("word/document.xml", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p><w:r><w:t>이사회 결의를 통해 채권 발행을 승인한다.</w:t></w:r></w:p>
        </w:body></w:document>"#
            .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();
}

fn write_pptx(dir: &Path) {
    let file = fs::File::create(dir.join("발표자료.pptx")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let slides = ["2026년 3분기 채권 발행 계획", "예상 발행 규모 및 일정"];
    for (i, text) in slides.iter().enumerate() {
        let name = format!("ppt/slides/slide{}.xml", i + 1);
        zip.start_file(&name, SimpleFileOptions::default()).unwrap();
        zip.write_all(
            format!(
                r#"<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:txBody>
                    <a:p><a:r><a:t>{text}</a:t></a:r></a:p>
                </p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
            )
            .as_bytes(),
        )
        .unwrap();
    }
    zip.finish().unwrap();
}

fn write_pdf(dir: &Path) {
    // A real PDF generated with LibreOffice headless (embedded Korean CID font) — for manually
    // verifying the section `docs/06_Development_Roadmap.md` flags as B1's highest-risk area.
    // Downgraded to META if libpdfium is unavailable.
    const KOREAN_PDF: &[u8] = include_bytes!("../tests/fixtures/korean.pdf");
    fs::write(dir.join("검토의견.pdf"), KOREAN_PDF).unwrap();
}

fn write_skip_cases(dir: &Path) {
    // Archive file — excluded by extension (PRD Chapter 3 default exclusion rules)
    fs::write(
        dir.join("보관용.zip"),
        b"not a real zip, extension-only test",
    )
    .unwrap();
    // Temp file — pattern of temp files created when Office saves a document
    fs::write(dir.join("~$규정.txt"), b"temp file placeholder").unwrap();
    // Corrupted PDF — extension is supported but parsing fails → verifies META(PARSE_FAIL) downgrade
    fs::write(dir.join("손상.pdf"), b"%PDF-1.4 not a real pdf body").unwrap();
}
