//! 파일 하나(또는 폴더 전체)를 계층(FULL/META/SKIP)에 맞춰 색인한다.

use rusqlite::Connection;
use std::path::Path;

use super::{canonical_path, IndexError, IndexService};
use crate::config::Config;
use crate::db::documents::{
    DemotionReason, DocumentRecord, DocumentRepository, IndexStatus, IndexTier, PathRecord,
};
use crate::db::search_repo::SearchRepository;
use crate::db::store::{DocumentStore, SqliteDocumentStore};
use crate::extract::{ContentExtractor, DocumentInfo};
use crate::nlp::{join_tokens, Tokenizer};
use crate::scan::{filter, hash, walker};

#[derive(Debug, Clone, Copy, Default)]
pub struct IndexOutcome {
    pub full: u64,
    pub meta: u64,
    pub skip: u64,
}

impl IndexOutcome {
    fn add(&mut self, tier: IndexTier) {
        match tier {
            IndexTier::Full => self.full += 1,
            IndexTier::Meta => self.meta += 1,
            IndexTier::Skip => self.skip += 1,
        }
    }
}

pub struct IndexPipeline<'a> {
    pub conn: &'a Connection,
    pub config: &'a Config,
    pub extractors: &'a [Box<dyn ContentExtractor>],
    /// 기본 토크나이저 — 항상 실행되어 `content_fts.morph`를 채운다.
    pub bigram: &'a dyn Tokenizer,
    /// 보조 토크나이저 — 가능할 때만 실행되어 `content_fts.morph_kiwi`를 채운다.
    /// `None`이면 `morph_kiwi`는 빈 문자열로 남는다.
    pub kiwi: Option<&'a dyn Tokenizer>,
}

impl<'a> IndexPipeline<'a> {
    /// `root` 아래 파일 전체를 스캔해 색인하고, 계층별 건수를 반환한다.
    pub fn index_directory(&self, root: &Path) -> Result<IndexOutcome, IndexError> {
        let mut outcome = IndexOutcome::default();
        for path in walker::scan(root) {
            let tier = self.index_file(&path)?;
            outcome.add(tier);
        }
        Ok(outcome)
    }

    /// 파일 하나를 색인하고 결정된 계층을 반환한다.
    pub fn index_file(&self, path: &Path) -> Result<IndexTier, IndexError> {
        // 상대/절대 경로 표현 차이로 같은 파일이 다른 문서로 색인되는 걸 막는다
        // (`canonical_path` 참조).
        let path = &canonical_path(path);
        let metadata = std::fs::metadata(path)?;
        let file_size = metadata.len();

        if filter::check(path, file_size, self.config).is_some() {
            return Ok(IndexTier::Skip);
        }

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let Some(extractor) = self.extractors.iter().find(|e| e.supports(&extension)) else {
            return Ok(IndexTier::Skip); // 미지원 포맷
        };

        let document_id = hash::hash_file(path)?;
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let modified_at = metadata.modified().ok().map(format_system_time);

        let tier = match DocumentRepository::get_tier(self.conn, &document_id)? {
            // 동일 내용의 문서가 이미 색인되어 있으면 재추출하지 않는다.
            Some(existing_tier) => existing_tier,
            None => self.extract_and_index(
                &document_id,
                path,
                &extension,
                file_size,
                extractor.as_ref(),
            )?,
        };

        DocumentRepository::upsert_path(
            self.conn,
            &PathRecord {
                path: path.to_string_lossy().to_string(),
                document_id: document_id.clone(),
                filename: filename.clone(),
                extension,
                modified_at,
            },
        )?;
        SearchRepository::index_filename(self.conn, &path.to_string_lossy(), &filename)?;

        Ok(tier)
    }

    fn extract_and_index(
        &self,
        document_id: &str,
        path: &Path,
        extension: &str,
        file_size: u64,
        extractor: &dyn ContentExtractor,
    ) -> Result<IndexTier, IndexError> {
        let document_info = DocumentInfo {
            path: path.to_path_buf(),
            extension: extension.to_string(),
        };

        match extractor.extract(&document_info) {
            Ok(result) => {
                let morph = join_tokens(&self.bigram.tokenize(&result.body));
                let morph_kiwi = self
                    .kiwi
                    .map(|kiwi| join_tokens(&kiwi.tokenize(&result.body)))
                    .unwrap_or_default();

                DocumentRepository::upsert_document(
                    self.conn,
                    &DocumentRecord {
                        document_id: document_id.to_string(),
                        file_size: file_size as i64,
                        text_bytes: result.body.len() as i64,
                        index_tier: IndexTier::Full,
                        index_status: IndexStatus::Indexed,
                        demotion_reason: None,
                    },
                )?;
                SqliteDocumentStore { conn: self.conn }.put_body(document_id, &result.body)?;
                SearchRepository::index_content(
                    self.conn,
                    document_id,
                    &result.body,
                    &morph,
                    &morph_kiwi,
                )?;
                Ok(IndexTier::Full)
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "추출 실패, META로 강등");
                DocumentRepository::upsert_document(
                    self.conn,
                    &DocumentRecord {
                        document_id: document_id.to_string(),
                        file_size: file_size as i64,
                        text_bytes: 0,
                        index_tier: IndexTier::Meta,
                        index_status: IndexStatus::MetaIndexed,
                        demotion_reason: Some(DemotionReason::ParseFail),
                    },
                )?;
                Ok(IndexTier::Meta)
            }
        }
    }
}

impl<'a> IndexService for IndexPipeline<'a> {
    fn index_document(&self, path: &Path) -> Result<(), IndexError> {
        self.index_file(path).map(|_| ())
    }
}

fn format_system_time(t: std::time::SystemTime) -> String {
    time::OffsetDateTime::from(t)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}
