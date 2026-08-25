//! Helpers shared by both `cli` binaries (`main.rs`'s `knowdesk-cli` and
//! `bin/find.rs`'s `kdfind`).

use knowdesk_core::extract::ooxml::{DocxExtractor, PptxExtractor};
use knowdesk_core::extract::pdf::PdfExtractor;
use knowdesk_core::extract::txt::TxtExtractor;
use knowdesk_core::extract::xlsx::XlsxExtractor;
use knowdesk_core::extract::ContentExtractor;

pub fn default_extractors() -> Vec<Box<dyn ContentExtractor>> {
    vec![
        Box::new(TxtExtractor),
        Box::new(XlsxExtractor),
        Box::new(DocxExtractor),
        Box::new(PptxExtractor),
        Box::new(PdfExtractor),
    ]
}

/// Same extractors as `default_extractors`, but as `Send + Sync` trait
/// objects - needed by `kdfind`'s `parallel_index` (shared across worker
/// threads via `std::thread::scope`), which a plain `Box<dyn
/// ContentExtractor>` can't be. Kept separate from `default_extractors`
/// itself (rather than widening its signature) so `knowdesk-cli`'s existing
/// single-threaded use of it is completely untouched.
pub fn default_extractors_sync() -> Vec<Box<dyn ContentExtractor + Send + Sync>> {
    vec![
        Box::new(TxtExtractor),
        Box::new(XlsxExtractor),
        Box::new(DocxExtractor),
        Box::new(PptxExtractor),
        Box::new(PdfExtractor),
    ]
}
