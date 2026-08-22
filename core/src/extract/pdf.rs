//! PDF extraction (`pdfium-render`). Loads and uses the native PDFium dynamic library.
//!
//! Pdfium can only be bound once per process — calling `Pdfium::new` a second time
//! panics. So we initialize it once via a global `OnceLock` and reuse it on later calls.
//!
//! The library path is set via the `KNOWDESK_PDFIUM_LIB_DIR` environment variable (e.g.
//! an extracted `lib/` directory). If unset, it falls back to the system library path.
//! In distributed builds, the installer bundles the native library alongside the
//! executable (`03_Architecture.md`).

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use pdfium_render::prelude::*;
use std::sync::OnceLock;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

pub struct PdfExtractor;

impl PdfExtractor {
    /// Checks whether libpdfium loaded successfully. Used to decide whether to skip
    /// tests in environments without the native library (e.g. CI).
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
                "Failed to load PDFium library: {e}"
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

    /// This test only passes if a real libpdfium is available at `KNOWDESK_PDFIUM_LIB_DIR`
    /// (or the system path). Since the native library may be missing in CI/dev
    /// environments, the test is skipped on load failure (so `cargo test` doesn't fail
    /// due to environment issues).
    #[test]
    fn extracts_text_when_pdfium_available() {
        if !PdfExtractor::is_available() {
            eprintln!("Skipping: libpdfium not found (KNOWDESK_PDFIUM_LIB_DIR not set)");
            return;
        }

        let sample =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/korean.pdf");
        if !sample.exists() {
            eprintln!("Skipping: sample PDF not found: {}", sample.display());
            return;
        }

        let result = PdfExtractor
            .extract(&DocumentInfo {
                path: sample,
                extension: "pdf".into(),
            })
            .unwrap();

        assert!(result.body.contains("채권"), "body: {}", result.body);
    }

    #[test]
    fn supports_only_pdf() {
        assert!(PdfExtractor.supports("pdf"));
        assert!(PdfExtractor.supports("PDF"));
        assert!(!PdfExtractor.supports("txt"));
    }
}
