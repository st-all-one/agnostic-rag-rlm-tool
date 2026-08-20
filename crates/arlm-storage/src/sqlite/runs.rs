use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::params;

use super::conn::Storage;
pub use super::nodes::FlatNode;

/// Stored run record.
#[derive(Debug, Clone)]
pub struct StoredRun {
    pub id: String,
    pub task: String,
    pub backend: Option<String>,
    pub mode: Option<String>,
    pub status: Option<String>,
    pub agent: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub total_cost: f64,
    pub total_tokens: i64,
    pub total_calls: i64,
    pub max_depth: Option<i64>,
    pub nodes_visited: Option<i64>,
    pub partial_answer: Option<String>,
    pub error: Option<String>,
}

/// Model usage record for a run.
#[derive(Debug, Clone)]
pub struct ModelUsage {
    pub run_id: String,
    pub model: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
}

/// Node call record.
#[derive(Debug, Clone)]
pub struct NodeCall {
    pub id: i64,
    pub run_id: String,
    pub node_id: String,
    pub depth: Option<i64>,
    pub node_type: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost: Option<f64>,
    pub duration_ms: Option<i64>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
}

/// Stored trajectory.
#[derive(Debug, Clone)]
pub struct StoredTrajectory {
    pub id: i64,
    pub project_name: String,
    pub agent: Option<String>,
    pub root_json: String,
    pub task: String,
    pub task_hash: Option<String>,
    pub total_cost: f64,
    pub created_at: i64,
}

const RUN_COLUMNS: &str = "id, task, backend, mode, status, agent, started_at, finished_at, duration_ms, total_cost, total_tokens, total_calls, max_depth, nodes_visited, partial_answer, error";

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRun> {
    Ok(StoredRun {
        id: row.get(0)?,
        task: row.get(1)?,
        backend: row.get(2)?,
        mode: row.get(3)?,
        status: row.get(4)?,
        agent: row.get(5)?,
        started_at: row.get(6)?,
        finished_at: row.get(7)?,
        duration_ms: row.get(8)?,
        total_cost: row.get(9)?,
        total_tokens: row.get(10)?,
        total_calls: row.get(11)?,
        max_depth: row.get(12)?,
        nodes_visited: row.get(13)?,
        partial_answer: row.get(14)?,
        error: row.get(15)?,
    })
}

/// Flat representation of a node for persistence (see [`FlatNode`]).
impl Storage {
    /// Insert a completed run and its associated node calls and model usage.
    pub fn insert_run(
        &self,
        run_id: &str,
        task: &str,
        backend: &str,
        mode: &str,
        status: &str,
        agent: &str,
        started_at_ms: u64,
        duration_ms: u64,
        total_cost: f64,
        total_tokens: u32,
        total_calls: u32,
        max_depth: u32,
        nodes_visited: u32,
        partial_answer: Option<&str>,
        error: Option<&str>,
        root_node: Option<&FlatNode>,
    ) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        let tx = conn.unchecked_transaction()?;

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        tx.execute(
            "INSERT INTO runs (id, task, backend, mode, status, agent, started_at, finished_at, duration_ms, total_cost, total_tokens, total_calls, max_depth, nodes_visited, partial_answer, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                run_id,
                task,
                backend,
                mode,
                status,
                agent,
                (started_at_ms / 1000) as i64,
                ((started_at_ms + duration_ms) / 1000) as i64,
                duration_ms as i64,
                total_cost,
                total_tokens as i64,
                total_calls as i64,
                max_depth as i64,
                nodes_visited as i64,
                partial_answer,
                error,
            ],
        )
        .context("failed to insert run")?;

        // Insert node calls
        if let Some(root) = root_node {
            let mut all_nodes = Vec::new();
            FlatNode::flatten(root, &mut all_nodes);
            for node in &all_nodes {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let duration = node
                    .finished_at_ms
                    .map(|f| f.saturating_sub(node.started_at_ms) as i64);

                tx.execute(
                    "INSERT INTO node_calls (run_id, node_id, depth, node_type, model, input_tokens, output_tokens, cost, duration_ms, status, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        run_id,
                        node.node_id,
                        node.depth as i64,
                        node.node_type,
                        None::<String>,
                        node.tokens as i64,
                        0i64,
                        node.cost_usd,
                        duration,
                        node.status,
                        node.error,
                    ],
                )
                .context("failed to insert node call")?;
            }

            // Aggregate model usage
            let mut model_usage: HashMap<String, (u32, u32, f64)> = HashMap::new();
            for node in &all_nodes {
                let entry = model_usage
                    .entry("unknown".to_string())
                    .or_insert((0, 0, 0.0));
                entry.0 += 1;
                entry.1 += node.tokens;
                entry.2 += node.cost_usd;
            }
            for (model, (calls, tokens, cost)) in &model_usage {
                tx.execute(
                    "INSERT OR REPLACE INTO run_model_usage (run_id, model, calls, input_tokens, output_tokens, cost) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![run_id, model, *calls as i64, *tokens as i64, 0i64, *cost],
                )
                .context("failed to insert model usage")?;
            }
        }

        tx.commit()?;
        tracing::info!(run_id, "persisted run");
        Ok(())
    }

    /// Get a run by ID.
    pub fn get_run(&self, run_id: &str) -> Result<Option<StoredRun>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"))
            .context("failed to prepare get_run query")?;

        let mut rows = stmt.query_map(params![run_id], row_to_run)?;

        rows.next().transpose().context("failed to get run")
    }

    /// Cancel a run by setting status to 'cancelled'.
    pub fn cancel_run(&self, run_id: &str) -> Result<()> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "UPDATE runs SET status = 'cancelled', finished_at = unixepoch() WHERE id = ?1",
            params![run_id],
        )
        .context("failed to cancel run")?;

        Ok(())
    }

    /// List recent runs.
    pub fn list_runs(&self, limit: i64) -> Result<Vec<StoredRun>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM runs ORDER BY started_at DESC LIMIT ?1"
            ))
            .context("failed to prepare list_runs query")?;

        let rows = stmt
            .query_map(params![limit], row_to_run)?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Get model usage for a run.
    pub fn get_run_model_usage(&self, run_id: &str) -> Result<Vec<ModelUsage>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare("SELECT run_id, model, calls, input_tokens, output_tokens, cost FROM run_model_usage WHERE run_id = ?1")
            .context("failed to prepare get_run_model_usage query")?;

        let rows = stmt
            .query_map(params![run_id], |row| {
                Ok(ModelUsage {
                    run_id: row.get(0)?,
                    model: row.get(1)?,
                    calls: row.get(2)?,
                    input_tokens: row.get(3)?,
                    output_tokens: row.get(4)?,
                    cost: row.get(5)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }

    /// Get total cost across all runs.
    pub fn total_cost(&self) -> Result<f64> {
        let conn = self.conn();
        let conn = conn.lock();

        let result: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(total_cost), 0.0) FROM runs",
                [],
                |row| row.get(0),
            )
            .context("failed to get total cost")?;

        Ok(result)
    }

    /// Get cost for a specific run.
    pub fn run_cost(&self, run_id: &str) -> Result<f64> {
        let conn = self.conn();
        let conn = conn.lock();

        let result: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost), 0.0) FROM run_model_usage WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .context("failed to get run cost")?;

        Ok(result)
    }

    /// Insert a trajectory for strategy reuse.
    pub fn insert_trajectory(
        &self,
        project_name: &str,
        agent: Option<&str>,
        root_json: &str,
        task: &str,
        task_hash: Option<&str>,
        total_cost: f64,
    ) -> Result<i64> {
        let conn = self.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO trajectories (project_name, agent, root_json, task, task_hash, total_cost) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![project_name, agent, root_json, task, task_hash, total_cost],
        )
        .context("failed to insert trajectory")?;

        Ok(conn.last_insert_rowid())
    }

    /// Get trajectories matching a task hash.
    pub fn get_trajectories_by_task_hash(&self, task_hash: &str) -> Result<Vec<StoredTrajectory>> {
        let conn = self.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, agent, root_json, task, task_hash, total_cost, created_at FROM trajectories WHERE task_hash = ?1 ORDER BY created_at DESC LIMIT 5",
            )
            .context("failed to prepare get_trajectories query")?;

        let rows = stmt
            .query_map(params![task_hash], |row| {
                Ok(StoredTrajectory {
                    id: row.get(0)?,
                    project_name: row.get(1)?,
                    agent: row.get(2)?,
                    root_json: row.get(3)?,
                    task: row.get(4)?,
                    task_hash: row.get(5)?,
                    total_cost: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }
}
