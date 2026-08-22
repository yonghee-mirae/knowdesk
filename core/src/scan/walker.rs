//! Directory scanning (`DirectoryScanner`).

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Recursively lists all regular file paths under `root`. Symbolic links are not followed.
pub fn scan(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect()
}
