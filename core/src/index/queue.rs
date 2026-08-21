//! 디바운스된 파일 변경 이벤트를 색인 파이프라인에 반영한다 (Phase B4).

use std::path::{Path, PathBuf};

use super::pipeline::IndexPipeline;
use super::{canonical_path, IndexError};
use crate::db::documents::{DocumentRepository, IndexTier};

/// 경로 하나가 어떻게 처리됐는지 (로그/테스트용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchOutcome {
    /// 생성/수정으로 판단해 색인했다.
    Indexed(IndexTier),
    /// 삭제로 판단해 색인에서 지웠다.
    Removed,
    /// 원래 색인 대상이 아니었다 (임시파일이 색인 전에 사라진 경우 등).
    Ignored,
}

/// 디바운스된 경로 묶음을 하나씩 처리한다.
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

/// 경로 하나를 처리한다. `notify`는 create/modify/remove를 구분해주지만, rename
/// 등 애매한 경우까지 안전하게 다루기 위해 지금 그 경로에 파일이 실제로 있는지로
/// 최종 판단한다.
pub fn handle_path(pipeline: &IndexPipeline, path: &Path) -> Result<WatchOutcome, IndexError> {
    if path.is_dir() {
        // 디렉터리 자체 이벤트는 색인 대상이 아니다 — 안의 파일들이 개별
        // 이벤트로 따로 들어온다.
        return Ok(WatchOutcome::Ignored);
    }

    if path.exists() {
        // index_file 내부에서 canonicalize한다.
        let tier = pipeline.index_file(path)?;
        Ok(WatchOutcome::Indexed(tier))
    } else {
        // 파일이 이미 없어져 canonicalize를 못 하므로, 부모 디렉터리 기준으로
        // 색인 시점과 같은 표현을 재구성한다 (`canonical_path` 참조).
        let path = canonical_path(path);
        match DocumentRepository::remove_path(pipeline.conn, &path.to_string_lossy())? {
            Some(_) => Ok(WatchOutcome::Removed),
            None => Ok(WatchOutcome::Ignored),
        }
    }
}
