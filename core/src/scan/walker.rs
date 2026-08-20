//! 폴더 스캔 (`DirectoryScanner`).

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// `root` 아래 모든 일반 파일 경로를 재귀적으로 나열한다. 심볼릭 링크는 따라가지 않는다.
pub fn scan(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect()
}
