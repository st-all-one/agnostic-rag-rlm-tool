use anyhow::{Context, Result};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[include_str!("../../migrations/001_initial.sql")];

/// Run all pending migrations.
///
/// # Errors
///
/// Returns an error if the schema version table cannot be created, the current
/// version cannot be read, a migration script fails, or the version record
/// cannot be inserted.
pub fn run_migrations(conn: &Connection) -> Result<()> {
    // Create schema_version table if it doesn't exist
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER DEFAULT (unixepoch())
        );",
    )
    .context("failed to create schema_version table")?;

    // Get current version
    let current_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .context("failed to get schema version")?;

    // Apply pending migrations
    #[allow(clippy::cast_possible_wrap)]
    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let version = i as i64 + 1;
        if version > current_version {
            conn.execute_batch(migration)
                .with_context(|| format!("failed to apply migration {version}"))?;

            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .context("failed to record migration")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_migrations_run() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();

        run_migrations(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"chunks".to_string()));
        assert!(tables.contains(&"buffers".to_string()));
        assert!(tables.contains(&"tasks".to_string()));
        assert!(tables.contains(&"findings".to_string()));
        assert!(tables.contains(&"history".to_string()));
        assert!(tables.contains(&"patterns".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
    }

    #[test]
    fn test_migrations_are_idempotent() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let conn = Connection::open(&db_path).unwrap();

        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 1);
    }
}
