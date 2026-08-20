//! 기본 제외 규칙 (`docs/01_KnowDesk_PRD.md` 3장 "기본 제외 규칙").

use crate::config::{Config, DEFAULT_EXCLUDED_EXTENSIONS, DEFAULT_TEMP_PATTERNS};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    CompressedFile,
    TempFile,
    OversizedFile,
}

/// 파일을 색인 대상에서 제외해야 하면 사유를 반환하고, 아니면 `None`을 반환한다.
pub fn check(path: &Path, file_size: u64, config: &Config) -> Option<SkipReason> {
    let filename = path.file_name()?.to_string_lossy().to_lowercase();

    if DEFAULT_TEMP_PATTERNS
        .iter()
        .any(|pat| filename.starts_with(pat) || filename.ends_with(pat))
    {
        return Some(SkipReason::TempFile);
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if DEFAULT_EXCLUDED_EXTENSIONS
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
