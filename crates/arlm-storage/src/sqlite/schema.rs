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
    include_str!("../../migrations/011_add_uuid_to_buffers.sql"),
    include_str!("../../migrations/012_add_summaries.sql"),
    include_str!("../../migrations/013_server_handlers.sql"),
    include_str!("../../migrations/014_add_summaries_fts.sql"),
    include_str!("../../migrations/015_add_auth.sql"),
    include_str!("../../migrations/016_add_qa_cache.sql"),
];

/// Total number of migrations in [`MIGRATIONS`].
pub const MIGRATION_COUNT: usize = MIGRATIONS.len();

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
