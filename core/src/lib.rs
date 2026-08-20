//! KnowDesk 순수 비즈니스 로직. Tauri를 참조하지 않는다 (`CLAUDE.md` 아키텍처 규칙).

pub mod config;
pub mod db;
pub mod extract;
pub mod index;
pub mod nlp;
pub mod scan;
pub mod search;

/// `DocumentID = SHA256(Content)` (`docs/01_KnowDesk_PRD.md` 7장).
pub type DocId = String;
