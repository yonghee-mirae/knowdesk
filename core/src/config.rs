//! Configuration system. Reads `settings.json`, falling back to defaults if absent
//! (`src-tauri`'s `settings_path()` decides where that file actually lives).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default exclusion rules (see PRD Chapter 3 "Default Exclusion Rules").
///
/// Neither this nor `DEFAULT_EXCLUDED_EXTENSIONS` (removed) has a `Config`
/// field - both are fixed, not user-configurable via `settings.json`
/// (2026-08-24 decision): the temp-file patterns are a known, stable set with
/// nothing meaningful for a user to tune, and the supported-format allowlist
/// (docx/xlsx/pptx/pdf/txt/md, `core::index::pipeline`'s registered
/// `ContentExtractor`s) already makes a separate extension denylist
/// redundant. `core/src/scan/filter.rs` applies this constant directly.
pub const DEFAULT_MAX_FILE_SIZE_MB: u64 = 50;
pub const DEFAULT_TEMP_PATTERNS: &[&str] = &["~$", ".tmp", ".temp", ".cache"];
/// `CmdOrCtrl` resolves to `⌘+Option+K` on macOS, `Ctrl+Alt+K` on Windows/Linux
/// (`src-tauri`'s `register_hotkey` parses this string).
///
/// ⚠️ O-7 (`KnowDesk_추가검토사항.md`) - whether global-hotkey hooking is
/// allowed by IT security policy on the target Windows fleet - is still
/// unconfirmed. This default is provisional pending that review.
pub const DEFAULT_HOTKEY: &str = "CmdOrCtrl+Alt+K";
/// Number of hits shown per search (`frontend/src/main.ts`'s search call).
/// `0` means unlimited (`core::search::SearchRequest::limit`'s doc comment) -
/// the default is unlimited.
pub const DEFAULT_RESULT_LIMIT: u32 = 0;
/// Delay after the last keystroke before a search actually fires
/// (`frontend/src/main.ts`'s `scheduleSearch`).
pub const DEFAULT_SEARCH_DEBOUNCE_MS: u32 = 150;
/// Quiet period after the last file-system event before a change to a watched
/// folder is treated as settled and actually indexed
/// (`core::index::watcher::FileWatcher`'s `debounce`). Same value
/// `knowdesk-cli`'s `watch` subcommand already uses as its `--debounce-ms`
/// default.
pub const DEFAULT_FILE_WATCH_DEBOUNCE_MS: u32 = 3000;

/// UI color theme (`frontend/src/core/theme.ts` applies it) - a plain data
/// value, not a UI concept `core` needs to know the meaning of. `System`
/// (the default) means "no explicit override, follow the OS setting", same
/// as never having set a theme at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    Light,
    #[default]
    System,
}

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
    /// (TASK-704 — replaced with a "설정 파일 열기" action instead of a
    /// Settings Window, `src-tauri`'s `open_settings_file`) - populated by
    /// hand-editing `settings.json` in a text editor. Empty by default:
    /// nothing is indexed until at least one folder is listed here.
    pub watched_folders: Vec<PathBuf>,
    pub theme: Theme,
    /// Global show/hide hotkey, in `tauri-plugin-global-shortcut`'s string
    /// syntax (`src-tauri`'s `register_hotkey`). Applied live - changing this
    /// and saving re-registers the hotkey without restarting the app.
    pub hotkey: String,
    /// Number of hits shown per search. `0` means unlimited.
    pub result_limit: u32,
    /// Delay (ms) after the last keystroke before a search actually fires.
    pub search_debounce_ms: u32,
    /// Quiet period (ms) after the last file-system event before a change to
    /// a watched folder is indexed. Applied live - changing this and saving
    /// takes effect on the very next settle, no restart needed
    /// (`core::index::watcher::FileWatcher::set_debounce`). Only the watcher
    /// for `watched_folders` uses this - the separate one that watches
    /// `settings.json` itself (so edits/deletes apply automatically) stays
    /// fixed, since tuning that one isn't a user-facing concern.
    pub file_watch_debounce_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("knowdesk.db"),
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB,
            watched_folders: Vec::new(),
            theme: Theme::default(),
            hotkey: DEFAULT_HOTKEY.to_string(),
            result_limit: DEFAULT_RESULT_LIMIT,
            search_debounce_ms: DEFAULT_SEARCH_DEBOUNCE_MS,
            file_watch_debounce_ms: DEFAULT_FILE_WATCH_DEBOUNCE_MS,
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
    fn theme_defaults_to_system_and_roundtrips_as_lowercase_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        assert_eq!(Config::default().theme, Theme::System);

        let config = Config {
            theme: Theme::Dark,
            ..Config::default()
        };
        config.save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""theme": "dark""#), "got: {text}");

        let reloaded = Config::load(Some(&path)).unwrap();
        assert_eq!(reloaded.theme, Theme::Dark);
    }

    #[test]
    fn hotkey_and_result_limit_default_and_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        assert_eq!(Config::default().hotkey, DEFAULT_HOTKEY);
        assert_eq!(Config::default().result_limit, DEFAULT_RESULT_LIMIT);
        assert_eq!(
            Config::default().search_debounce_ms,
            DEFAULT_SEARCH_DEBOUNCE_MS
        );
        assert_eq!(
            Config::default().file_watch_debounce_ms,
            DEFAULT_FILE_WATCH_DEBOUNCE_MS
        );

        let config = Config {
            hotkey: "CmdOrCtrl+Shift+Space".to_string(),
            result_limit: 50,
            search_debounce_ms: 300,
            file_watch_debounce_ms: 1000,
            ..Config::default()
        };
        config.save(&path).unwrap();

        let reloaded = Config::load(Some(&path)).unwrap();
        assert_eq!(reloaded.hotkey, "CmdOrCtrl+Shift+Space");
        assert_eq!(reloaded.result_limit, 50);
        assert_eq!(reloaded.search_debounce_ms, 300);
        assert_eq!(reloaded.file_watch_debounce_ms, 1000);
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
