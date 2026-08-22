pub mod documents;
pub mod migrate;
pub mod schema;
pub mod search_repo;
pub mod store;

use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

/// Index DB handle. Returned with migrations already applied.
pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        // Without this, two connections opening the same not-yet-created DB file at
        // nearly the same moment (e.g. `src-tauri`'s `SearchWorker` and index worker
        // both starting up) can hit `SQLITE_BUSY` immediately instead of just waiting
        // for the other to finish its schema migration - confirmed in practice.
        // rusqlite's default busy handler is a no-op (fails immediately), so this has
        // to be set explicitly.
        conn.busy_timeout(Duration::from_secs(5))?;
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
