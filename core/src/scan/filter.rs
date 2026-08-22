//! Exclusion rules (`docs/01_KnowDesk_PRD.md` Chapter 3 "Default Exclusion Rules").
//!
//! Temp-file patterns (`DEFAULT_TEMP_PATTERNS`) are applied directly as a fixed
//! constant, not a `Config` field - not user-configurable via `settings.json`
//! (2026-08-24 decision: a known, stable set with nothing meaningful for a user
//! to tune). There used to also be an `excluded_extensions` denylist here
//! (zip/7z/rar), but it was redundant with `core::index::pipeline`'s own
//! extractor-registry check: any extension with no registered `ContentExtractor`
//! is already `SKIP`ped there regardless of this module (the supported-format
//! list is now a fixed allowlist: docx/xlsx/pptx/pdf/txt/md). Temp-file patterns
//! aren't redundant the same way, since a transient file like `~$notes.docx` has
//! an otherwise-supported extension - only a filename check catches it.

use crate::config::{Config, DEFAULT_TEMP_PATTERNS};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    TempFile,
    OversizedFile,
}

/// Returns a reason if the file should be excluded from indexing, or `None` otherwise.
pub fn check(path: &Path, file_size: u64, config: &Config) -> Option<SkipReason> {
    let filename = path.file_name()?.to_string_lossy().to_lowercase();

    if DEFAULT_TEMP_PATTERNS
        .iter()
        .any(|pat| filename.starts_with(pat) || filename.ends_with(pat))
    {
        return Some(SkipReason::TempFile);
    }

    if file_size > config.max_file_size_bytes() {
        return Some(SkipReason::OversizedFile);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn flags_temp_files() {
        assert_eq!(
            check(&PathBuf::from("~$notes.docx"), 10, &cfg()),
            Some(SkipReason::TempFile)
        );
        assert_eq!(
            check(&PathBuf::from("report.tmp"), 10, &cfg()),
            Some(SkipReason::TempFile)
        );
    }

    #[test]
    fn flags_oversized_files() {
        let over = cfg().max_file_size_bytes() + 1;
        assert_eq!(
            check(&PathBuf::from("big.txt"), over, &cfg()),
            Some(SkipReason::OversizedFile)
        );
    }

    #[test]
    fn allows_normal_files() {
        assert_eq!(check(&PathBuf::from("report.txt"), 10, &cfg()), None);
    }
}
