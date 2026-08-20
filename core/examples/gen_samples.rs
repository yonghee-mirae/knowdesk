//! 수동 테스트용 샘플 폴더 생성기.
//!
//! 실행:
//! ```text
//! cargo run -p knowdesk-core --example gen_samples [출력 경로, 기본값 ./samples]
//! ```
//!
//! 지금까지 구현된 범위(Phase A + Phase B1 XLSX/DOCX/PPTX/PDF)를 한 번에 수동 검증할 수 있게
//! 정상 케이스와 제외 규칙 케이스를 함께 만든다. 생성물은 매번 재생성 가능하므로 git에는
//! 커밋하지 않는다 (`.gitignore` 참조).
//!
//! PDF 추출은 네이티브 libpdfium이 있어야 실제로 동작한다. `KNOWDESK_PDFIUM_LIB_DIR`
//! 환경 변수로 라이브러리 경로를 지정하지 않으면 `검토의견.pdf`는 META로 강등된다
//! (오류가 아니라 이 프로젝트의 정상적인 강등 정책이다).

use std::fs;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./samples".to_string());
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("출력 폴더 생성 실패");

    write_txt_utf8(out_dir);
    write_txt_euckr(out_dir);
    write_txt_unrelated(out_dir);
    write_xlsx(out_dir);
    write_docx(out_dir);
    write_pptx(out_dir);
    write_pdf(out_dir);
    write_skip_cases(out_dir);

    println!("샘플 생성 완료: {}", out_dir.display());
    println!();
    println!("검증 방법:");
    println!(
        "  cargo run -p knowdesk-cli -- --db ./samples.db index {}",
        out_dir.display()
    );
    println!("  cargo run -p knowdesk-cli -- --db ./samples.db search \"채권 발행\"");
}

fn write_txt_utf8(dir: &Path) {
    fs::write(
        dir.join("규정.txt"),
        "본 문서는 채권 발행 절차를 규정한다.\n채권 발행 시 이사회 승인이 필요하다.\n",
    )
    .unwrap();
}

fn write_txt_euckr(dir: &Path) {
    // 인코딩 자동 감지(encoding_rs + chardetng) 확인용 — CP949/EUC-KR 저장 문서 대응.
    let (bytes, _, had_errors) =
        encoding_rs::EUC_KR.encode("회의록: 채권 발행 계획을 다음 분기로 연기한다.");
    assert!(!had_errors);
    fs::write(dir.join("회의록_EUCKR.txt"), bytes).unwrap();
}

fn write_txt_unrelated(dir: &Path) {
    fs::write(dir.join("무관.txt"), "회의록 요약: 다음 분기 예산안 검토\n").unwrap();
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
    // LibreOffice headless로 만든 실제 PDF (한글 CID 폰트 임베딩) — `docs/06_Development_Roadmap.md`
    // B1이 가장 리스크가 크다고 지목한 구간의 수동 검증용. libpdfium이 없으면 META로 강등된다.
    const KOREAN_PDF: &[u8] = include_bytes!("../tests/fixtures/korean.pdf");
    fs::write(dir.join("검토의견.pdf"), KOREAN_PDF).unwrap();
}

fn write_skip_cases(dir: &Path) {
    // 압축 파일 — 확장자 기준 제외 (PRD 3장 기본 제외 규칙)
    fs::write(
        dir.join("보관용.zip"),
        b"not a real zip, extension-only test",
    )
    .unwrap();
    // 임시 파일 — Office 저장 시 생기는 임시파일 패턴
    fs::write(dir.join("~$규정.txt"), b"temp file placeholder").unwrap();
    // 손상된 PDF — 확장자는 지원 대상이지만 파싱 실패 → META(PARSE_FAIL) 강등 확인용
    fs::write(dir.join("손상.pdf"), b"%PDF-1.4 not a real pdf body").unwrap();
}
