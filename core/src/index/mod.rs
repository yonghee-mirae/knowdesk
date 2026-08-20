//! `IndexService` — 색인 파이프라인 (`docs/08_API_Contracts.md`).

pub mod pipeline;

use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum IndexError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
}

pub trait IndexService {
    fn index_document(&self, path: &Path) -> Result<(), IndexError>;
}
