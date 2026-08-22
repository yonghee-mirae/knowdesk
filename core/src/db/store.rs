//! `DocumentStore` — abstraction over how original text is stored (see `docs/08_API_Contracts.md`).
//! The initial configuration stores the original text as-is; this trait's signature is kept
//! stable so it won't need to change even if storage switches to a compressed form.

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
        // Insert a minimal row first to satisfy the documents FK constraint.
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
