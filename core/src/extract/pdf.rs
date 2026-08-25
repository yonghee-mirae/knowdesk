//! PDF extraction (`pdfium-render`). Loads and uses the native PDFium dynamic library.
//!
//! Pdfium can only be bound once per process — calling `Pdfium::new` a second time
//! panics. So we initialize it once via a global `OnceLock` and reuse it on later calls.
//!
//! The library path is set via the `KNOWDESK_PDFIUM_LIB_DIR` environment variable (e.g.
//! an extracted `lib/` directory). If unset, it falls back to the system library path.
//! In distributed builds, the installer bundles the native library alongside the
//! executable (`03_Architecture.md`). A caller can instead call `PdfExtractor::set_lib_dir`
//! to decide the path itself and bypass the environment variable entirely (`cli`'s
//! `kdfind`, which reads it from `settings_cli.json` instead).

use super::{ContentExtractor, DocumentInfo, ExtractError, ExtractionResult};
use pdfium_render::prelude::*;
use std::path::PathBuf;
use std::sync::OnceLock;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();
/// Set once via `set_lib_dir` by a caller that resolves the library path itself
/// (e.g. `cli`'s `kdfind`, from `settings_cli.json`) instead of the
/// `KNOWDESK_PDFIUM_LIB_DIR` environment variable. `None` (the default, if
/// `set_lib_dir` is never called) means "use the env var / system library
/// fallback below, as before".
static PDFIUM_LIB_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

pub struct PdfExtractor;

impl PdfExtractor {
    /// Explicitly decides the PDFium library directory, bypassing
    /// `KNOWDESK_PDFIUM_LIB_DIR` entirely - for callers that resolve the path from
    /// their own config rather than an environment variable. Must be called before
    /// the first `extract`/`is_available` call (whichever runs first wins, same as
    /// any other `OnceLock`); a no-op on later calls.
    pub fn set_lib_dir(dir: Option<PathBuf>) {
        let _ = PDFIUM_LIB_DIR.set(dir);
    }

    /// Checks whether libpdfium loaded successfully. Used to decide whether to skip
    /// tests in environments without the native library (e.g. CI).
    pub fn is_available() -> bool {
        Self::pdfium().is_ok()
    }

    fn pdfium() -> Result<&'static Pdfium, ExtractError> {
        let result = PDFIUM.get_or_init(|| {
            let bindings = match PDFIUM_LIB_DIR.get() {
                Some(Some(dir)) => {
                    Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir))
                }
                Some(None) => Pdfium::bind_to_system_library(),
                None => match std::env::var("KNOWDESK_PDFIUM_LIB_DIR") {
                    Ok(dir) => {
                        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&dir))
                    }
                    Err(_) => Pdfium::bind_to_system_library(),
                },
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
