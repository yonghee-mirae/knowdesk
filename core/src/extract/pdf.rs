//! PDF 추출 (`pdfium-render`). 네이티브 PDFium 동적 라이브러리를 로드해 사용한다.
//!
//! Pdfium은 프로세스당 한 번만 바인딩할 수 있다 — 두 번째로 `Pdfium::new`를 호출하면
//! panic한다. 그래서 전역 `OnceLock`으로 한 번만 초기화하고 이후 호출은 재사용한다.
//!
//! 라이브러리 경로는 `KNOWDESK_PDFIUM_LIB_DIR` 환경 변수로 지정한다 (예: 압축 해제한
//! `lib/` 디렉터리). 지정하지 않으면 시스템 라이브러리 경로에서 찾는다. 배포판에서는
//! 인스톨러가 네이티브 라이브러리를 실행 파일과 함께 동봉한다 (`03_Architecture.md`).

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use pdfium_render::prelude::*;
use std::sync::OnceLock;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

pub struct PdfExtractor;

impl PdfExtractor {
    /// libpdfium 로드에 성공했는지 확인한다. 네이티브 라이브러리가 없는 환경(CI 등)에서
    /// 테스트를 건너뛸지 판단하는 용도로 쓴다.
    pub fn is_available() -> bool {
        Self::pdfium().is_ok()
    }

    fn pdfium() -> Result<&'static Pdfium, ExtractError> {
        let result = PDFIUM.get_or_init(|| {
            let bindings = match std::env::var("KNOWDESK_PDFIUM_LIB_DIR") {
                Ok(dir) => {
                    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
                }
                Err(_) => Pdfium::bind_to_system_library(),
            };
            bindings.map(Pdfium::new).map_err(|e| e.to_string())
        });

        match result {
            Ok(pdfium) => Ok(pdfium),
            Err(e) => Err(ExtractError::Parse(format!(
                "PDFium 라이브러리 로드 실패: {e}"
            ))),
        }
    }
}

impl ContentExtractor for PdfExtractor {
    fn supports(&self, ext: &str) -> bool {
        ext.eq_ignore_ascii_case("pdf")
    }

    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError> {
        let pdfium = Self::pdfium()?;
        let doc = pdfium
            .load_pdf_from_file(&document.path, None)
            .map_err(|e| ExtractError::Parse(e.to_string()))?;

        let mut body = String::new();
        for page in doc.pages().iter() {
            let text = page
                .text()
                .map_err(|e| ExtractError::Parse(e.to_string()))?;
            body.push_str(&text.all());
            body.push('\n');
        }

        Ok(ExtractionResult { body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 이 테스트는 `KNOWDESK_PDFIUM_LIB_DIR`(또는 시스템 경로)에 실제 libpdfium이
    /// 있어야 통과한다. CI/개발 환경에 네이티브 라이브러리가 없을 수 있으므로,
    /// 로드 실패 시 테스트를 건너뛴다 (`cargo test`가 환경 문제로 실패하지 않게).
    #[test]
    fn extracts_text_when_pdfium_available() {
        if !PdfExtractor::is_available() {
            eprintln!("libpdfium을 찾을 수 없어 건너뜁니다 (KNOWDESK_PDFIUM_LIB_DIR 미설정)");
            return;
        }

        let sample =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/korean.pdf");
        if !sample.exists() {
            eprintln!("샘플 PDF가 없어 건너뜁니다: {}", sample.display());
            return;
        }

        let result = PdfExtractor
            .extract(&DocumentInfo {
                path: sample,
                extension: "pdf".into(),
            })
            .unwrap();

        assert!(result.body.contains("채권"), "본문: {}", result.body);
    }

    #[test]
    fn supports_only_pdf() {
        assert!(PdfExtractor.supports("pdf"));
        assert!(PdfExtractor.supports("PDF"));
        assert!(!PdfExtractor.supports("txt"));
    }
}
