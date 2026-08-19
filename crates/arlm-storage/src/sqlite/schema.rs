use anyhow::{Context, Result};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_initial.sql"),
    include_str!("../../migrations/004_add_runs_cost.sql"),
    include_str!("../../migrations/005_add_trajectories.sql"),
    include_str!("../../migrations/006_add_sessions.sql"),
    include_str!("../../migrations/007_add_result_cache.sql"),
    include_str!("../../migrations/008_add_events.sql"),
    include_str!("../../migrations/009_add_entities.sql"),
    include_str!("../../migrations/010_add_last_accessed_at.sql"),
];

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

    // Run ANALYZE after migrations for accurate query planning stats
    conn.execute_batch("ANALYZE;")
        .context("failed to run ANALYZE")?;

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
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(tables.contains(&"chunks".to_string()));
        assert!(tables.contains(&"buffers".to_string()));
        assert!(tables.contains(&"tasks".to_string()));
        assert!(tables.contains(&"findings".to_string()));
        assert!(tables.contains(&"history".to_string()));
        assert!(tables.contains(&"patterns".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
        assert!(tables.contains(&"runs".to_string()));
        assert!(tables.contains(&"run_model_usage".to_string()));
        assert!(tables.contains(&"node_calls".to_string()));
        assert!(tables.contains(&"trajectories".to_string()));
        assert!(tables.contains(&"sessions".to_string()));
        assert!(tables.contains(&"session_contexts".to_string()));
        assert!(tables.contains(&"session_history".to_string()));
        assert!(tables.contains(&"result_cache".to_string()));
        assert!(tables.contains(&"events".to_string()));
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
        assert_eq!(version, 8);
    }
}
