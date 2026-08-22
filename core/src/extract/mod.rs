//! `ContentExtractor` — content extraction abstraction (`docs/08_API_Contracts.md`).
//! Phase A implements only TXT. XLSX/DOCX/PPTX/PDF are Phase B.

pub mod ooxml;
pub mod pdf;
pub mod txt;
pub mod xlsx;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DocumentInfo {
    pub path: PathBuf,
    pub extension: String,
}

#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub body: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ExtractError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported extension: {0}")]
    Unsupported(String),
    #[error("parse error: {0}")]
    Parse(String),
}

pub trait ContentExtractor {
    fn supports(&self, ext: &str) -> bool;
    fn extract(&self, document: &DocumentInfo) -> Result<ExtractionResult, ExtractError>;
}

/// Finds the extractor matching the extension. Returns `None` if none matches (caller handles it as SKIP).
pub fn find_extractor<'a>(
    extractors: &'a [Box<dyn ContentExtractor>],
    ext: &str,
) -> Option<&'a dyn ContentExtractor> {
    extractors
        .iter()
        .find(|extractor| extractor.supports(ext))
        .map(|b| b.as_ref())
}
