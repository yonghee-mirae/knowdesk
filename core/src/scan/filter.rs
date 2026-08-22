//! Exclusion rules (`docs/01_KnowDesk_PRD.md` Chapter 3 "Default Exclusion Rules").
//! The extension/temp-pattern lists themselves live on `Config` now
//! (`excluded_extensions`/`excluded_temp_patterns`), user-configurable via
//! `settings.json` - this module just applies whatever the current `Config` holds.

use crate::config::Config;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    CompressedFile,
    TempFile,
    OversizedFile,
}

/// Returns a reason if the file should be excluded from indexing, or `None` otherwise.
pub fn check(path: &Path, file_size: u64, config: &Config) -> Option<SkipReason> {
    let filename = path.file_name()?.to_string_lossy().to_lowercase();

    if config
        .excluded_temp_patterns
        .iter()
        .any(|pat| filename.starts_with(pat.as_str()) || filename.ends_with(pat.as_str()))
    {
        return Some(SkipReason::TempFile);
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if config
            .excluded_extensions
            .iter()
            .any(|excluded| excluded.eq_ignore_ascii_case(ext))
        {
            return Some(SkipReason::CompressedFile);
        }
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
    fn flags_compressed_extensions() {
        assert_eq!(
            check(&PathBuf::from("archive.zip"), 10, &cfg()),
            Some(SkipReason::CompressedFile)
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
