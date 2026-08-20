pub mod documents;
pub mod migrate;
pub mod schema;
pub mod search_repo;
pub mod store;

use rusqlite::Connection;
use std::path::Path;

/// 색인 DB 핸들. 마이그레이션까지 실행된 상태로 반환된다.
pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrate::run(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        migrate::run(&conn)?;
        Ok(Self { conn })
    }
}
