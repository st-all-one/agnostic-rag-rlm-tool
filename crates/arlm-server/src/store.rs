//! Typed, pool-safe data access for the gRPC handlers.
//!
//! The server runs the storage pool in `Pooled` mode, where the
//! single-connection helpers on `arlm_storage::Storage` would panic. Every
//! query here goes through [`arlm_storage::Storage::connection`], which works
//! in both single and pooled modes.

use anyhow::{Context, Result};
use arlm_proto::proto::RunStatus as ProtoRunStatus;
use arlm_storage::Storage;
use rusqlite::{OptionalExtension, params};

/// Project (buffer) row.
#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: i64,
    pub uuid: Option<String>,
    pub name: String,
    pub path: String,
    pub total_chunks: i64,
    pub total_files: i64,
    pub created_at: i64,
}

/// Session row.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub project: String,
    pub title: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

/// Run row used by the server handlers.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: String,
    pub project: Option<String>,
    pub task: String,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub status: String,
    pub answer: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub total_tokens: i64,
    pub total_cost: f64,
    pub nodes_visited: i64,
    pub max_depth: i64,
}

const PROJECT_COLUMNS: &str = "id, uuid, name, path, total_chunks, total_files, created_at";

const RUN_COLUMNS: &str = "id, project, task, backend, model, status, partial_answer, started_at, finished_at, duration_ms, total_tokens, total_cost, nodes_visited, max_depth";

fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
    Ok(ProjectRow {
        id: row.get(0)?,
        uuid: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        total_chunks: row.get(4)?,
        total_files: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        project: row.get(1)?,
        task: row.get(2)?,
        backend: row.get(3)?,
        model: row.get(4)?,
        status: row.get(5)?,
        answer: row.get(6)?,
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        duration_ms: row.get(9)?,
        total_tokens: row.get(10)?,
        total_cost: row.get(11)?,
        nodes_visited: row.get::<_, Option<i64>>(12)?.unwrap_or(0),
        max_depth: row.get::<_, Option<i64>>(13)?.unwrap_or(0),
    })
}

/// Map a persisted run status string to the proto enum.
#[must_use]
pub fn proto_run_status(status: &str) -> ProtoRunStatus {
    match status {
        "running" => ProtoRunStatus::StatusRunning,
        "completed" => ProtoRunStatus::StatusCompleted,
        "failed" => ProtoRunStatus::StatusFailed,
        "cancelled" | "canceled" => ProtoRunStatus::StatusCancelled,
        _ => ProtoRunStatus::StatusPending,
    }
}

/// Map the proto run status enum to a persisted status string.
#[must_use]
pub fn db_run_status(status: ProtoRunStatus) -> &'static str {
    match status {
        ProtoRunStatus::StatusRunning => "running",
        ProtoRunStatus::StatusCompleted => "completed",
        ProtoRunStatus::StatusFailed => "failed",
        ProtoRunStatus::StatusCancelled => "cancelled",
        ProtoRunStatus::StatusPending => "pending",
    }
}

// ── Projects ────────────────────────────────────────────────────────────────

/// Insert a project (buffer) and return its numeric id.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_project(storage: &Storage, name: &str, path: &str) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let uuid = uuid::Uuid::now_v7().to_string();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO buffers (name, path, uuid) VALUES (?1, ?2, ?3)",
            params![name, path, uuid],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .context("failed to insert project")
}

/// Look up a project by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_name(storage: &Storage, name: &str) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE name = ?1"
            ))
            .context("failed to prepare get_project_by_name query")?;
        let mut rows = stmt.query_map(params![name], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by name")
}

/// Look up a project by numeric id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_id(storage: &Storage, id: i64) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE id = ?1"
            ))
            .context("failed to prepare get_project_by_id query")?;
        let mut rows = stmt.query_map(params![id], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by id")
}

/// Look up a project by uuid.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_project_by_uuid(storage: &Storage, uuid: &str) -> Result<Option<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers WHERE uuid = ?1"
            ))
            .context("failed to prepare get_project_by_uuid query")?;
        let mut rows = stmt.query_map(params![uuid], row_to_project)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to query project by uuid")
}

/// List every project ordered by name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_projects(storage: &Storage) -> Result<Vec<ProjectRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {PROJECT_COLUMNS} FROM buffers ORDER BY name"
            ))
            .context("failed to prepare list_projects query")?;
        let rows = stmt
            .query_map([], row_to_project)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
    .context("failed to list projects")
}

// ── Sessions ───────────────────────────────────────────────────────────────

/// Insert a session for a project.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_session(storage: &Storage, id: &str, project: &str, title: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = chrono::Utc::now().timestamp();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO sessions (id, project_name, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, project, title, now],
        )?;
        Ok(())
    })
    .context("failed to insert session")
}

/// List sessions for a project, newest first.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn list_sessions(storage: &Storage, project: &str) -> Result<Vec<SessionRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, title, created_at, updated_at \
                 FROM sessions WHERE project_name = ?1 ORDER BY created_at DESC",
            )
            .context("failed to prepare list_sessions query")?;
        let rows = stmt
            .query_map(params![project], row_to_session)?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(rows)
    })
    .context("failed to list sessions")
}

/// Get a single session by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_session(storage: &Storage, id: &str) -> Result<Option<SessionRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, title, created_at, updated_at \
                 FROM sessions WHERE id = ?1",
            )
            .context("failed to prepare get_session query")?;
        let mut rows = stmt.query_map(params![id], row_to_session)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to get session")
}

/// Count the number of turns stored for a session.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_session_turns(storage: &Storage, session_id: &str) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM session_history WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?)
    })
    .context("failed to count session turns")
}

/// Persist a session turn (query/result pair) and bump `updated_at`.
///
/// # Errors
///
/// Returns an error if the insert or update fails.
pub fn insert_session_turn(
    storage: &Storage,
    session_id: &str,
    query: &str,
    result: &str,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;
    let now = chrono::Utc::now().timestamp();

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO session_history (session_id, query, result) VALUES (?1, ?2, ?3)",
            params![session_id, query, result],
        )?;
        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    })
    .context("failed to insert session turn")
}

// ── Runs ───────────────────────────────────────────────────────────────────

/// Insert a run record in its initial (`running`) state.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_run(storage: &Storage, run: &RunRow) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO runs (id, project, task, backend, model, status, started_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                run.id,
                run.project,
                run.task,
                run.backend,
                run.model,
                run.status,
                run.started_at,
            ],
        )?;
        Ok(())
    })
    .context("failed to insert run")
}

/// Fetch a run by id.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn get_run(storage: &Storage, run_id: &str) -> Result<Option<RunRow>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        let mut stmt = conn
            .prepare(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"))
            .context("failed to prepare get_run query")?;
        let mut rows = stmt.query_map(params![run_id], row_to_run)?;
        Ok(rows.next().transpose()?)
    })
    .context("failed to get run")
}

/// Cancel a running run.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn cancel_run(storage: &Storage, run_id: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'cancelled', finished_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().timestamp(), run_id],
        )?;
        Ok(())
    })
    .context("failed to cancel run")
}

/// Mark a run as completed with its final result.
///
/// # Errors
///
/// Returns an error if the update fails.
#[allow(clippy::too_many_arguments)]
pub fn complete_run(
    storage: &Storage,
    run_id: &str,
    answer: &str,
    duration_ms: u64,
    nodes_visited: u32,
    max_depth: u32,
    total_tokens: u64,
    total_cost: f64,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'completed', partial_answer = ?1, finished_at = ?2, \
             duration_ms = ?3, nodes_visited = ?4, max_depth = ?5, total_tokens = ?6, \
             total_cost = ?7 WHERE id = ?8",
            params![
                answer,
                chrono::Utc::now().timestamp(),
                u64::try_from(duration_ms).unwrap_or(u64::MAX),
                u64::try_from(nodes_visited).unwrap_or(u64::MAX),
                u64::try_from(max_depth).unwrap_or(u64::MAX),
                u64::try_from(total_tokens).unwrap_or(u64::MAX),
                total_cost,
                run_id,
            ],
        )?;
        Ok(())
    })
    .context("failed to complete run")
}

/// Mark a run as failed with an error message.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn fail_run(storage: &Storage, run_id: &str, error: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE runs SET status = 'failed', partial_answer = ?1, finished_at = ?2 \
             WHERE id = ?3",
            params![error, chrono::Utc::now().timestamp(), run_id,],
        )?;
        Ok(())
    })
    .context("failed to fail run")
}

/// Count runs currently in the `running` state.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_active_runs(storage: &Storage) -> Result<u32> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(u32::try_from(
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE status = 'running'",
                [],
                |row| row.get::<_, i64>(0),
            )?
            .max(0),
        )
        .unwrap_or(u32::MAX))
    })
    .context("failed to count active runs")
}

// ── Summaries ──────────────────────────────────────────────────────────────

/// Run a store operation on the blocking pool.
///
/// All SQLite access in async contexts should go through this helper so the
/// async runtime is never blocked on pool acquisition or I/O.
///
/// # Errors
///
/// Returns an error if the operation fails or the blocking task panics.
pub async fn blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .context("blocking store task panicked")?
}

// ── Indexing persistence ────────────────────────────────────────────────────

/// Insert a chunk row using the real `chunks` schema and return its id.
///
/// # Errors
///
/// Returns an error if the insert fails.
#[allow(clippy::too_many_arguments)]
pub fn insert_chunk(
    storage: &Storage,
    buffer_id: i64,
    file_path: &str,
    line_start: i32,
    line_end: i32,
    hash_bytes: &[u8],
    language: Option<&str>,
    chunk_type: Option<&str>,
    token_count: Option<i64>,
) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunks (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, language, chunk_type, token_count) \
             VALUES (?1, ?2, 0, 0, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                buffer_id,
                file_path,
                line_start,
                line_end,
                hash_bytes,
                language,
                chunk_type,
                token_count,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    })
    .context("failed to insert chunk")
}

/// Insert chunk text into `chunk_texts`.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_chunk_text(storage: &Storage, chunk_id: i64, content: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
            params![chunk_id, content],
        )?;
        Ok(())
    })
    .context("failed to insert chunk text")
}

/// Index a chunk in the FTS5 table (`rowid` links back to `chunks.id`).
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_fts_row(storage: &Storage, chunk_id: i64, content: &str) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO chunks_fts(rowid, content) VALUES (?1, ?2)",
            params![chunk_id, content],
        )?;
        Ok(())
    })
    .context("failed to index chunk in FTS")
}

/// Store extracted entities for a chunk.
///
/// # Errors
///
/// Returns an error if any of the inserts fail.
pub fn insert_entities(storage: &Storage, chunk_id: i64, entities: &[String]) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    for entity in entities {
        conn.execute(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO chunk_entities (chunk_id, entity) VALUES (?1, ?2)",
                params![chunk_id, entity],
            )?;
            conn.execute(
                "INSERT INTO entities_fts (entity) VALUES (?1)",
                params![entity],
            )?;
            Ok(())
        })?;
    }

    Ok(())
}

/// Update the aggregate counts on a buffer after an indexing pass.
///
/// # Errors
///
/// Returns an error if the update fails.
pub fn update_buffer_counts(
    storage: &Storage,
    buffer_id: i64,
    total_chunks: i64,
    total_files: i64,
    embedding_model: &str,
    embedding_dims: i64,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "UPDATE buffers SET total_chunks = ?1, total_files = ?2, embedding_model = ?3, embedding_dims = ?4, last_indexed_at = unixepoch() \
             WHERE id = ?5",
            params![
                total_chunks,
                total_files,
                embedding_model,
                embedding_dims,
                buffer_id,
            ],
        )?;
        Ok(())
    })
    .context("failed to update buffer counts")
}

/// Summary counts for a project, grouped by scope.
#[derive(Debug, Clone, Default)]
pub struct SummaryCounts {
    pub total: i64,
    pub file: i64,
    pub module: i64,
    pub project: i64,
}

/// Count summaries for a project by scope.
///
/// # Errors
///
/// Returns an error if any query fails.
pub fn summary_counts(storage: &Storage, project: &str) -> Result<SummaryCounts> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        const SCOPE_BASE: &str =
            "SELECT COUNT(*) FROM summaries WHERE buffer_id IN (SELECT id FROM buffers WHERE name = ?1)";
        let count_scope = |conn: &rusqlite::Connection, scope: &str| -> rusqlite::Result<i64> {
            if scope.is_empty() {
                conn.query_row(SCOPE_BASE, params![project], |row| row.get(0))
            } else {
                conn.query_row(
                    &format!("{SCOPE_BASE} AND scope = ?2"),
                    params![project, scope],
                    |row| row.get(0),
                )
            }
        };

        let total = count_scope(conn, "").unwrap_or(0);
        let file = count_scope(conn, "file").unwrap_or(0);
        let module = count_scope(conn, "module").unwrap_or(0);
        let project_count = count_scope(conn, "project").unwrap_or(0);
        Ok(SummaryCounts {
            total,
            file,
            module,
            project: project_count,
        })
    })
    .context("failed to count summaries")
}

/// Total number of summaries across all projects.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn count_all_summaries(storage: &Storage) -> Result<i64> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM summaries", [], |row| row.get(0))?))
        .context("failed to count all summaries")
}

/// Buffer id for a project name.
///
/// # Errors
///
/// Returns an error if the query fails.
pub fn buffer_id_for_project(storage: &Storage, project: &str) -> Result<Option<i64>> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM buffers WHERE name = ?1 OR uuid = ?1",
                params![project],
                |row| row.get(0),
            )
            .optional()?)
    })
    .context("failed to find buffer for project")
}

/// Insert a hierarchical summary record.
///
/// # Errors
///
/// Returns an error if the insert fails.
pub fn insert_summary(
    storage: &Storage,
    buffer_id: i64,
    content: &str,
    scope: &str,
    source_chunk_ids: &str,
    source_hash: &str,
    confidence: f64,
    tokens: u32,
) -> Result<()> {
    let conn = storage
        .connection()
        .context("failed to acquire connection")?;

    conn.execute(|conn| {
        conn.execute(
            "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                buffer_id,
                content,
                scope,
                source_chunk_ids,
                source_hash,
                confidence,
                tokens,
            ],
        )?;
        Ok(())
    })
    .context("failed to insert summary")
}
