//! `settings_cli.json` - `kdfind`'s own config file (`docs/13_CLI_Tool.md`).
//!
//! Deliberately separate from `knowdesk_core::config::Config` (the GUI's
//! `settings.json`) - that struct is full of fields `kdfind` has no use for
//! (`watched_folders`/`theme`/`hotkey`/...), and `kdfind` needs a couple of fields
//! `Config` doesn't have at all (the native library paths below). `kdfind` reads
//! `KNOWDESK_*` environment variables for nothing - unlike `knowdesk-cli`, it's
//! meant to be distributed standalone, so every native library path is configured
//! through this one file instead.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CliConfig {
    /// Whether to attempt loading the Kiwi morphological analyzer at all. Off by
    /// default, same reasoning as `Config::enable_morphological_analysis` (Kiwi
    /// costs several hundred MB of RSS once loaded).
    pub enable_morphological_analysis: bool,
    /// Path to the native Kiwi library (e.g. `libkiwi.so`/`.dylib`/`kiwi.dll`).
    /// Kiwi is only loaded when this, `kiwi_model_dir`, and
    /// `enable_morphological_analysis` are all set/true.
    pub kiwi_lib_path: Option<PathBuf>,
    /// Path to the Kiwi model directory.
    pub kiwi_model_dir: Option<PathBuf>,
    /// Path to the native PDFium library directory. If unset, PDF extraction is
    /// skipped and PDFs are indexed as metadata-only (filename search still
    /// works, content search doesn't).
    pub pdfium_lib_dir: Option<PathBuf>,
}

impl CliConfig {
    /// If a `settings_cli.json` file exists at `path`, reads and merges it;
    /// otherwise returns the default (every field omitted from the JSON keeps its
    /// default too, `#[serde(default)]` above).
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&text)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Writes this config to `path` as pretty-printed JSON, creating the parent
    /// directory if it doesn't exist yet (the app-data directory may not exist on
    /// a first run).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// `settings_cli.json`'s fixed location - same app-data folder as the GUI's
/// `settings.json` (`knowdesk_core::config::app_data_dir()`). No override flag or
/// environment variable - `kdfind` is meant to just work out of the box for a
/// standalone distribution, without the caller needing to know where its own
/// config lives.
pub fn cli_settings_path() -> PathBuf {
    knowdesk_core::config::app_data_dir().join("settings_cli.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_default_when_file_is_missing() {
        let config = CliConfig::load(Path::new("/nonexistent/settings_cli.json")).unwrap();
        assert!(!config.enable_morphological_analysis);
        assert_eq!(config.kiwi_lib_path, None);
        assert_eq!(config.kiwi_model_dir, None);
        assert_eq!(config.pdfium_lib_dir, None);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        // Nested, not-yet-existing directory - `save()` must create it.
        let path = dir.path().join("nested").join("settings_cli.json");

        let config = CliConfig {
            enable_morphological_analysis: true,
            kiwi_lib_path: Some(PathBuf::from("/opt/kiwi/libkiwi.so")),
            kiwi_model_dir: Some(PathBuf::from("/opt/kiwi/models")),
            pdfium_lib_dir: Some(PathBuf::from("/opt/pdfium/lib")),
        };
        config.save(&path).unwrap();

        let reloaded = CliConfig::load(&path).unwrap();
        assert!(reloaded.enable_morphological_analysis);
        assert_eq!(
            reloaded.kiwi_lib_path,
            Some(PathBuf::from("/opt/kiwi/libkiwi.so"))
        );
        assert_eq!(
            reloaded.kiwi_model_dir,
            Some(PathBuf::from("/opt/kiwi/models"))
        );
        assert_eq!(
            reloaded.pdfium_lib_dir,
            Some(PathBuf::from("/opt/pdfium/lib"))
        );
    }

    #[test]
    fn partial_json_keeps_remaining_fields_at_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings_cli.json");
        std::fs::write(&path, r#"{"enable_morphological_analysis": true}"#).unwrap();

        let config = CliConfig::load(&path).unwrap();
        assert!(config.enable_morphological_analysis);
        assert_eq!(config.kiwi_lib_path, None);
        assert_eq!(config.pdfium_lib_dir, None);
    }
}
