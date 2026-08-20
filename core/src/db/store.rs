//! `DocumentStore` — 원문 저장 방식 추상화 (`docs/08_API_Contracts.md` 참조).
//! 초기 구성은 원문을 그대로 저장하며, 압축 저장으로 전환하더라도 이 trait의
//! 시그니처는 바뀌지 않게 유지한다.

use rusqlite::{params, Connection, OptionalExtension};

pub trait DocumentStore {
    fn put_body(&self, doc: &str, text: &str) -> rusqlite::Result<()>;
    fn get_body(&self, doc: &str) -> rusqlite::Result<Option<String>>;
}

pub struct SqliteDocumentStore<'a> {
    pub conn: &'a Connection,
}

impl<'a> DocumentStore for SqliteDocumentStore<'a> {
    fn put_body(&self, doc: &str, text: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO document_bodies (document_id, body) VALUES (?1, ?2)
             ON CONFLICT(document_id) DO UPDATE SET body = excluded.body",
            params![doc, text],
        )?;
        Ok(())
    }

    fn get_body(&self, doc: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT body FROM document_bodies WHERE document_id = ?1",
                params![doc],
                |row| row.get(0),
            )
            .optional()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn put_then_get_roundtrips() {
        let db = Db::open_in_memory().unwrap();
        // documents FK 제약을 만족시키기 위해 최소 행을 먼저 넣는다.
        db.conn
            .execute(
                "INSERT INTO documents (document_id, file_size, text_bytes, index_tier, index_status)
                 VALUES ('abc', 0, 0, 'FULL', 'INDEXED')",
                [],
            )
            .unwrap();

        let store = SqliteDocumentStore { conn: &db.conn };
        store.put_body("abc", "채권 발행 절차").unwrap();
        assert_eq!(
            store.get_body("abc").unwrap().as_deref(),
            Some("채권 발행 절차")
        );

        store.put_body("abc", "수정된 본문").unwrap();
        assert_eq!(
            store.get_body("abc").unwrap().as_deref(),
            Some("수정된 본문")
        );

        assert_eq!(store.get_body("missing").unwrap(), None);
    }
}
