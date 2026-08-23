//! Applies debounced file-change events to the indexing pipeline (Phase B4).

use std::path::{Path, PathBuf};

use super::pipeline::IndexPipeline;
use super::{canonical_path, IndexError};
use crate::db::documents::{DocumentRepository, IndexTier};

/// How a single path was handled (for logging/testing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchOutcome {
    /// Judged as a create/modify and indexed.
    Indexed(IndexTier),
    /// Judged as a delete and removed from the index.
    Removed,
    /// Was never an indexing target to begin with (e.g. a temp file that disappeared
    /// before it could be indexed).
    Ignored,
}

/// Processes a batch of debounced paths one at a time.
pub fn drain(
    pipeline: &IndexPipeline,
    paths: Vec<PathBuf>,
) -> Vec<(PathBuf, Result<WatchOutcome, IndexError>)> {
    paths
        .into_iter()
        .map(|path| {
            let outcome = handle_path(pipeline, &path);
            (path, outcome)
        })
        .collect()
}

/// Processes a single path. `notify` distinguishes create/modify/remove, but to
/// safely handle ambiguous cases like renames too, the final decision is made based
/// on whether a file actually exists at that path right now.
pub fn handle_path(pipeline: &IndexPipeline, path: &Path) -> Result<WatchOutcome, IndexError> {
    if path.is_dir() {
        // Events for the directory itself aren't indexing targets — the files inside
        // it arrive as their own separate events.
        return Ok(WatchOutcome::Ignored);
    }

    if path.exists() {
        // Canonicalized internally by index_file.
        let tier = pipeline.index_file(path)?;
        Ok(WatchOutcome::Indexed(tier))
    } else {
        // The file is already gone so it can't be canonicalized; reconstruct the same
        // representation used at index time, based on the nearest still-existing ancestor
        // (see `canonical_path`).
        let path = canonical_path(path);
        let path_str = path.to_string_lossy();
        let removed_exact = DocumentRepository::remove_path(pipeline.conn, &path_str)?;
        // `path` might have been a directory - a whole watched folder (or one nested
        // inside it) deleted at once, not just a single file - which `remove_path` alone
        // can't clean up (it only matches one exact `paths.path` value). This purges
        // anything nested under it; a no-op in the common single-file-delete case.
        let removed_nested = DocumentRepository::remove_paths_under(pipeline.conn, &path_str)?;
        if removed_exact.is_some() || !removed_nested.is_empty() {
            Ok(WatchOutcome::Removed)
        } else {
            Ok(WatchOutcome::Ignored)
        }
    }
}
