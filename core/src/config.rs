//! Configuration system. Reads `settings.json`, falling back to defaults if absent
//! (`src-tauri`'s `settings_path()` decides where that file actually lives).

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Default exclusion rules (see PRD Chapter 3 "Default Exclusion Rules")
pub const DEFAULT_MAX_FILE_SIZE_MB: u64 = 50;
pub const DEFAULT_EXCLUDED_EXTENSIONS: &[&str] = &["zip", "7z", "rar"];
pub const DEFAULT_TEMP_PATTERNS: &[&str] = &["~$", ".tmp", ".temp", ".cache"];

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub db_path: PathBuf,
    pub max_file_size_mb: u64,
    /// Folders to index and continuously watch (`docs/12_UI_Spec.md` C5's
    /// Settings Window mockup, TASK-704 — registering folders through that UI
    /// isn't built yet, so today this is only populated by hand-editing
    /// `settings.json`). Empty by default: nothing is indexed until at least
    /// one folder is listed here.
    pub watched_folders: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("knowdesk.db"),
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB,
            watched_folders: Vec::new(),
        }
    }
}

impl Config {
    /// If a `settings.json` file exists at `path`, reads and merges it; otherwise
    /// returns the default (every field omitted from the JSON keeps its default too -
    /// see `#[serde(default)]` above).
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        match path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p)?;
                Ok(serde_json::from_str(&text)?)
            }
            _ => Ok(Self::default()),
        }
    }

    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_mb * 1024 * 1024
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_default_when_file_is_missing() {
        let config = Config::load(Some(Path::new("/nonexistent/settings.json"))).unwrap();
        assert_eq!(config.watched_folders, Vec::<PathBuf>::new());
        assert_eq!(config.max_file_size_mb, DEFAULT_MAX_FILE_SIZE_MB);
    }

    #[test]
    fn load_reads_watched_folders_from_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"watched_folders": ["/a/이사회", "/b/리서치"]}"#).unwrap();

        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(
            config.watched_folders,
            vec![PathBuf::from("/a/이사회"), PathBuf::from("/b/리서치")]
        );
        // Fields omitted from the JSON keep their defaults.
        assert_eq!(config.max_file_size_mb, DEFAULT_MAX_FILE_SIZE_MB);
    }
}
