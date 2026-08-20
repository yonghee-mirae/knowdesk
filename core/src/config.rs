//! 설정 시스템. TOML 파일을 읽되, 없으면 기본값을 사용한다.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 기본 제외 규칙 (PRD 3장 "기본 제외 규칙" 참조)
pub const DEFAULT_MAX_FILE_SIZE_MB: u64 = 50;
pub const DEFAULT_EXCLUDED_EXTENSIONS: &[&str] = &["zip", "7z", "rar"];
pub const DEFAULT_TEMP_PATTERNS: &[&str] = &["~$", ".tmp", ".temp", ".cache"];

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub db_path: PathBuf,
    pub max_file_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("knowdesk.db"),
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB,
        }
    }
}

impl Config {
    /// `path`에 설정 파일이 있으면 읽어 병합하고, 없으면 기본값을 반환한다.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        match path {
            Some(p) if p.exists() => {
                let text = std::fs::read_to_string(p)?;
                Ok(toml::from_str(&text)?)
            }
            _ => Ok(Self::default()),
        }
    }

    pub fn max_file_size_bytes(&self) -> u64 {
        self.max_file_size_mb * 1024 * 1024
    }
}
