//! `IndexService` — indexing pipeline (`docs/08_API_Contracts.md`).

pub mod pipeline;
pub mod queue;
pub mod watcher;

use std::path::{Path, PathBuf};

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

/// Canonicalizes a path. The `paths` table uses the path string as its primary key,
/// but the same file can be represented as different strings depending on context —
/// in practice, `cli watch`'s initial full scan uses the path the user gave as-is
/// (e.g. `./samples/파일.txt`), but when `notify` reports a later change, it gives the
/// event as an absolute path with the current working directory prepended
/// (`/current/directory/./samples/파일.txt`). Treating the two as different files causes
/// the same file to be indexed as two separate documents, and a bug where old content
/// stays in search forever even after the content is edited (actually observed in
/// practice). If the file exists, canonicalize it directly; if not (a deleted file),
/// reconstruct it by canonicalizing the nearest ancestor that still exists and
/// re-appending everything below it - walking up more than one level matters when a
/// whole folder was deleted at once rather than just one file inside it, so the
/// file's immediate parent is gone too (see `queue::handle_path` and
/// `db::documents::DocumentRepository::remove_paths_under`, its counterpart for
/// purging every indexed path that was nested under a deleted directory).
pub fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let mut trailing = Vec::new();
    let mut ancestor = path;
    while let Some(parent) = ancestor.parent() {
        // `ancestor` always has a parent here, so it always has a file_name too.
        trailing.push(ancestor.file_name().expect("has a parent"));
        if let Ok(canonical_parent) = parent.canonicalize() {
            return trailing
                .into_iter()
                .rev()
                .fold(canonical_parent, |acc, name| acc.join(name));
        }
        ancestor = parent;
    }
    path.to_path_buf()
}
