//! Configuration system. Reads `settings.json`, falling back to defaults if absent
//! (`src-tauri`'s `settings_path()` decides where that file actually lives).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default exclusion rules (see PRD Chapter 3 "Default Exclusion Rules")
pub const DEFAULT_MAX_FILE_SIZE_MB: u64 = 50;
pub const DEFAULT_EXCLUDED_EXTENSIONS: &[&str] = &["zip", "7z", "rar"];
pub const DEFAULT_TEMP_PATTERNS: &[&str] = &["~$", ".tmp", ".temp", ".cache"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Only meaningful to `knowdesk-cli` (its `--db` flag sets this directly).
    /// Never written to or read from `settings.json` (`#[serde(skip)]`) - the
    /// GUI computes its own DB path independently (`src-tauri`'s `db_path()`:
    /// `KNOWDESK_DB_PATH` env var, else a fixed per-OS default) and never
    /// touches this field.
    #[serde(skip)]
    pub db_path: PathBuf,
    pub max_file_size_mb: u64,
    /// Folders to index and continuously watch. There's no in-app UI for this
    /// (TASK-704 — replaced with a "설정 파일 폴더 열기" action instead of a
    /// Settings Window, `src-tauri`'s `open_settings_folder`) - populated by
    /// hand-editing `settings.json` in a text editor. Empty by default:
    /// nothing is indexed until at least one folder is listed here.
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

    /// Writes this config to `path` as pretty-printed JSON, creating the parent
    /// directory if it doesn't exist yet (the app-data directory may not exist
    /// on a first run - `settings_path()`'s directory is the same one `db_path()`
    /// already creates, but `save()` shouldn't assume caller order).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
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

    #[test]
    fn save_then_load_roundtrips_watched_folders() {
        let dir = tempfile::tempdir().unwrap();
        // Nested, not-yet-existing directory - `save()` must create it.
        let path = dir.path().join("nested").join("settings.json");

        let config = Config {
            watched_folders: vec![PathBuf::from("/a/이사회")],
            ..Config::default()
        };
        config.save(&path).unwrap();

        let reloaded = Config::load(Some(&path)).unwrap();
        assert_eq!(reloaded.watched_folders, vec![PathBuf::from("/a/이사회")]);
    }

    #[test]
    fn save_never_writes_db_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        Config::default().save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("db_path"),
            "settings.json must not mention db_path - only KNOWDESK_DB_PATH/the built-in default control it: {text}"
        );

        // A hand-edited settings.json that *does* still have an old `db_path`
        // entry (from before this changed) must not error out - it's just
        // ignored, like any other unknown field.
        std::fs::write(
            &path,
            r#"{"db_path": "/somewhere/custom.db", "watched_folders": []}"#,
        )
        .unwrap();
        Config::load(Some(&path)).unwrap();
    }

    #[test]
    fn save_creates_file_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(!path.exists());

        Config::default().save(&path).unwrap();

        assert!(path.exists());
        let reloaded = Config::load(Some(&path)).unwrap();
        assert_eq!(reloaded.watched_folders, Vec::<PathBuf>::new());
        assert_eq!(reloaded.max_file_size_mb, DEFAULT_MAX_FILE_SIZE_MB);
    }
}
