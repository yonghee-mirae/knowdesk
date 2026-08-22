pub mod documents;
pub mod migrate;
pub mod schema;
pub mod search_repo;
pub mod store;

use rusqlite::Connection;
use std::path::Path;

/// Index DB handle. Returned with migrations already applied.
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
