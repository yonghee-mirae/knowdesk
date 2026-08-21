//! `IndexService` — 색인 파이프라인 (`docs/08_API_Contracts.md`).

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

/// 경로를 정규화한다. `paths` 테이블은 경로 문자열이 기본 키인데, 같은 파일이
/// 상황에 따라 다른 문자열로 표현될 수 있다 — 실제로 `cli watch`의 최초 전체
/// 스캔은 사용자가 준 경로를 그대로 쓰지만(예: `./samples/파일.txt`), `notify`가
/// 이후 변경을 알릴 땐 현재 작업 디렉터리를 붙인 절대 경로로 이벤트를 준다
/// (`/현재/디렉터리/./samples/파일.txt`). 둘을 다른 파일로 취급하면 같은 파일이
/// 문서 두 개로 나뉘어 색인되고, 내용을 수정해도 예전 내용이 영구히 검색에 남는
/// 버그가 생긴다(실제로 발견됨). 파일이 있으면 그대로 canonicalize하고, 없으면
/// (삭제된 파일) 부모 디렉터리만 canonicalize해서 재구성한다.
pub fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(file_name);
        }
    }
    path.to_path_buf()
}
