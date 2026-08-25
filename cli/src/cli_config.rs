//! `settings_cli.json` - `kdfind`'s own config file (`docs/13_CLI_Tool.md`).
//!
//! Deliberately separate from `knowdesk_core::config::Config` (the GUI's
//! `settings.json`) - that struct is full of fields `kdfind` has no use for
//! (`watched_folders`/`theme`/`hotkey`/...), and `kdfind` needs a couple of fields
//! `Config` doesn't have at all (the native library paths below). `kdfind` reads
//! `KNOWDESK_*` environment variables for nothing - unlike `knowdesk-cli`, it's
//! meant to be distributed standalone, so every native library path is configured
//! through this one file instead - or auto-detected relative to the installed
//! `.deb` package's layout when the file leaves them unset (`resolve_paths`).

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

    /// The native library paths this run should actually use: an explicit
    /// `settings_cli.json` value always wins; a field left `null` falls back to
    /// whatever's bundled at the `.deb` package's install layout (binary at
    /// `/usr/bin/kdfind`, libraries under `/usr/lib/kdfind/{pdfium,kiwi}`) - so a
    /// packaged install works without the user ever having to open this file,
    /// while still leaving it possible to point at a different build.
    /// `enable_morphological_analysis` is checked separately by the caller either
    /// way - Kiwi's own memory cost means it should never turn on just because
    /// bundled files happen to exist.
    pub fn resolve_paths(&self) -> ResolvedPaths {
        self.resolve_paths_from(bundled_lib_dir())
    }

    /// Testable core of `resolve_paths` — `base` is normally `bundled_lib_dir()`'s
    /// result, taken as a parameter so tests can point it at a temp directory
    /// instead of depending on the test binary's own `current_exe()`.
    fn resolve_paths_from(&self, base: Option<PathBuf>) -> ResolvedPaths {
        let pdfium_lib_dir = self.pdfium_lib_dir.clone().or_else(|| {
            let dir = base.as_ref()?.join("pdfium");
            dir.join("libpdfium.so").is_file().then_some(dir)
        });
        let kiwi_lib_path = self.kiwi_lib_path.clone().or_else(|| {
            base.as_ref()
                .map(|b| b.join("kiwi/libkiwi.so"))
                .filter(|p| p.is_file())
        });
        let kiwi_model_dir = self.kiwi_model_dir.clone().or_else(|| {
            base.as_ref()
                .map(|b| b.join("kiwi/models/cong/base"))
                .filter(|p| p.is_dir())
        });

        ResolvedPaths {
            pdfium_lib_dir,
            kiwi_lib_path,
            kiwi_model_dir,
        }
    }
}

/// Output of `CliConfig::resolve_paths` - same shape as the three path fields
/// on `CliConfig` itself, but with the bundled-install fallback already
/// applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPaths {
    pub pdfium_lib_dir: Option<PathBuf>,
    pub kiwi_lib_path: Option<PathBuf>,
    pub kiwi_model_dir: Option<PathBuf>,
}

/// The `.deb` package's layout: the binary installs to `/usr/bin/kdfind` and
/// the bundled native libraries to `/usr/lib/kdfind/{pdfium,kiwi}` - one
/// level up from the executable's own directory, then into `lib/kdfind`.
/// `None` if the executable's own path can't be determined
/// (`current_exe()` failing) or has no parent directory to climb from.
fn bundled_lib_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin_dir = exe.parent()?;
    let usr_dir = bin_dir.parent()?;
    Some(usr_dir.join("lib/kdfind"))
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

    /// Builds a fake `/usr/lib/kdfind/{pdfium,kiwi}` tree under a temp dir, in
    /// the exact shape a `.deb` install would produce.
    fn fake_bundled_install() -> tempfile::TempDir {
        let base = tempfile::tempdir().unwrap();
        let pdfium_dir = base.path().join("pdfium");
        std::fs::create_dir_all(&pdfium_dir).unwrap();
        std::fs::write(pdfium_dir.join("libpdfium.so"), "fake").unwrap();

        let kiwi_model_dir = base.path().join("kiwi/models/cong/base");
        std::fs::create_dir_all(&kiwi_model_dir).unwrap();
        std::fs::write(base.path().join("kiwi/libkiwi.so"), "fake").unwrap();

        base
    }

    #[test]
    fn resolve_paths_falls_back_to_bundled_install_when_unset() {
        let install = fake_bundled_install();
        let config = CliConfig::default();

        let resolved = config.resolve_paths_from(Some(install.path().to_path_buf()));

        assert_eq!(resolved.pdfium_lib_dir, Some(install.path().join("pdfium")));
        assert_eq!(
            resolved.kiwi_lib_path,
            Some(install.path().join("kiwi/libkiwi.so"))
        );
        assert_eq!(
            resolved.kiwi_model_dir,
            Some(install.path().join("kiwi/models/cong/base"))
        );
    }

    #[test]
    fn resolve_paths_prefers_an_explicit_settings_value_over_the_bundled_install() {
        let install = fake_bundled_install();
        let config = CliConfig {
            pdfium_lib_dir: Some(PathBuf::from("/custom/pdfium")),
            ..CliConfig::default()
        };

        let resolved = config.resolve_paths_from(Some(install.path().to_path_buf()));

        assert_eq!(
            resolved.pdfium_lib_dir,
            Some(PathBuf::from("/custom/pdfium"))
        );
        // The other two fields were left unset, so they still fall back.
        assert_eq!(
            resolved.kiwi_lib_path,
            Some(install.path().join("kiwi/libkiwi.so"))
        );
    }

    #[test]
    fn resolve_paths_ignores_a_bundled_dir_missing_the_expected_files() {
        // Directory exists but doesn't actually contain libpdfium.so/libkiwi.so -
        // must not report a path that doesn't work.
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(base.path().join("pdfium")).unwrap();
        let config = CliConfig::default();

        let resolved = config.resolve_paths_from(Some(base.path().to_path_buf()));

        assert_eq!(resolved.pdfium_lib_dir, None);
        assert_eq!(resolved.kiwi_lib_path, None);
        assert_eq!(resolved.kiwi_model_dir, None);
    }

    #[test]
    fn resolve_paths_with_no_bundled_install_and_no_explicit_config_is_all_none() {
        let config = CliConfig::default();
        let resolved = config.resolve_paths_from(None);
        assert_eq!(resolved, ResolvedPaths::default());
    }
}
