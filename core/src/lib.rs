//! KnowDesk pure business logic. Does not reference Tauri (see the `CLAUDE.md` architecture rules).

pub mod config;
pub mod db;
pub mod extract;
pub mod index;
pub mod nlp;
pub mod scan;
pub mod search;

/// `DocumentID = SHA256(Content)` (see `docs/01_KnowDesk_PRD.md` Chapter 7).
pub type DocId = String;
