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
    // Six `documents` columns that were never actually read or written by any code
    // path (`core/src/db/schema.rs`'s doc comment on `SCHEMA_V1` has the full
    // reasoning): `index_status` duplicated `index_tier` one-to-one, `demotion_reason`
    // only ever held `PARSE_FAIL` or nothing (distinguishing DRM/CORRUPT/ENCRYPTED was
    // decided unnecessary), and `drm_status`/`retry_count`/`last_attempt_at`/
    // `content_stored` backed features that were never built.
    (
        3,
        "ALTER TABLE documents DROP COLUMN index_status;
         ALTER TABLE documents DROP COLUMN demotion_reason;
         ALTER TABLE documents DROP COLUMN drm_status;
         ALTER TABLE documents DROP COLUMN retry_count;
         ALTER TABLE documents DROP COLUMN last_attempt_at;
         ALTER TABLE documents DROP COLUMN content_stored;",
    ),
    // A deleted file/folder (or one taken out of `watched_folders`) frees rows, but
    // SQLite never shrinks the database file on `DELETE` alone - the freed pages just
    // sit in the file's internal freelist for reuse, not returned to the OS. One-time
    // cleanup for whatever bloat already accumulated before `Db::reclaim_space` existed
    // to keep it from growing further - a cost paid once, by whichever connection
    // happens to open the DB first after upgrading.
    (4, "VACUUM;"),
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
        assert_eq!(version, 4);
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

    #[test]
    fn drops_unused_documents_columns_left_over_from_v1() {
        // v1 still creates these columns (historical record) - v3 must clean them
        // up since none of them was ever actually read or written by any code path
        // (`core/src/db/schema.rs`'s doc comment on `SCHEMA_V1` has the full reasoning).
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('documents')")
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        for dropped in [
            "index_status",
            "demotion_reason",
            "drm_status",
            "retry_count",
            "last_attempt_at",
            "content_stored",
        ] {
            assert!(
                !columns.contains(&dropped.to_string()),
                "{dropped} should have been dropped, remaining columns: {columns:?}"
            );
        }
        // Sanity check the columns that must still be there weren't dropped too.
        for kept in [
            "document_id",
            "file_size",
            "text_bytes",
            "index_tier",
            "indexed_at",
        ] {
            assert!(
                columns.contains(&kept.to_string()),
                "{kept} should still be there, remaining columns: {columns:?}"
            );
        }
    }
}
