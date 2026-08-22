//! Migration runner. `KnowDesk_추가검토사항.md` C-3 (whether rollback is needed) is still
//! undecided, so until that's settled, only up-migrations are implemented, per the decision
//! in `11_Implementation_Plan.md`.

use rusqlite::Connection;

use super::schema::SCHEMA_V1;

/// List of (version, SQL to apply). New migrations are appended at the end.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    // Filename search moved off FTS5 to a plain substring match on `paths.filename`
    // (`docs/05_Search_Language_v1.md`, Filename Mode) - `filename_fts` is no longer
    // read or written anywhere, so drop it.
    (2, "DROP TABLE IF EXISTS filename_fts;"),
];

pub fn run(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;

    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;

    for &(version, sql) in MIGRATIONS {
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, datetime('now'))",
            rusqlite::params![version],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_migrations_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap(); // running twice should not error

        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn drops_filename_fts_left_over_from_v1() {
        // v1 still creates `filename_fts` (historical record of what it actually
        // applied) - v2 must clean it up since filename search no longer uses it.
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'filename_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }
}
