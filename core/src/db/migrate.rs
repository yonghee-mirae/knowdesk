//! 마이그레이션 러너. `KnowDesk_추가검토사항.md` C-3(롤백 필요 여부)가 미결이므로,
//! 확정 전까지는 `11_Implementation_Plan.md`의 결정대로 up 마이그레이션만 구현한다.

use rusqlite::Connection;

use super::schema::SCHEMA_V1;

/// (버전, 적용할 SQL) 목록. 새 마이그레이션은 뒤에 추가한다.
const MIGRATIONS: &[(i64, &str)] = &[(1, SCHEMA_V1)];

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
        run(&conn).unwrap(); // 두 번 실행해도 에러 없어야 함

        let version: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }
}
