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

    /// Adds `folder` to `watched_folders` if it isn't already there. Returns
    /// whether it was newly added (`false` if it was already present).
    pub fn add_watched_folder(&mut self, folder: PathBuf) -> bool {
        if self.watched_folders.contains(&folder) {
            return false;
        }
        self.watched_folders.push(folder);
        true
    }

    /// Removes `folder` from `watched_folders`. Returns whether it was present
    /// (and therefore actually removed).
    pub fn remove_watched_folder(&mut self, folder: &Path) -> bool {
        let before = self.watched_folders.len();
        self.watched_folders.retain(|f| f != folder);
        self.watched_folders.len() != before
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

        let mut config = Config::default();
        config.add_watched_folder(PathBuf::from("/a/이사회"));
        config.save(&path).unwrap();

        let reloaded = Config::load(Some(&path)).unwrap();
        assert_eq!(reloaded.watched_folders, vec![PathBuf::from("/a/이사회")]);
    }

    #[test]
    fn add_watched_folder_dedupes() {
        let mut config = Config::default();
        assert!(config.add_watched_folder(PathBuf::from("/a")));
        assert!(
            !config.add_watched_folder(PathBuf::from("/a")),
            "adding the same folder twice should report it wasn't newly added"
        );
        assert_eq!(config.watched_folders, vec![PathBuf::from("/a")]);
    }

    #[test]
    fn remove_watched_folder_reports_whether_it_was_present() {
        let mut config = Config::default();
        config.add_watched_folder(PathBuf::from("/a"));

        assert!(config.remove_watched_folder(Path::new("/a")));
        assert!(config.watched_folders.is_empty());
        assert!(
            !config.remove_watched_folder(Path::new("/a")),
            "removing an already-absent folder should report false"
        );
    }
}
