//! Storage, retrieval, and replay implementations for [`TrajectoryEngine`].

use anyhow::{Context, Result};
use rusqlite::params;

use arlm_storage::Storage;

use super::serialize::{compute_task_hash, flatten_decomposition};
use super::{DecompositionNode, FindSimilarOptions, RunTrajectory, TrajectoryEngine};
use crate::ScopedTimer;

impl TrajectoryEngine {
    /// Create a new `TrajectoryEngine`.
    ///
    /// Ensures the trajectories schema exists.
    ///
    /// # Errors
    ///
    /// Returns an error if schema creation fails.
    pub fn new(storage: Storage) -> Result<Self> {
        Self::ensure_schema(&storage)?;
        Ok(Self { storage })
    }

    fn ensure_schema(storage: &Storage) -> Result<()> {
        let conn = storage.conn();
        let conn = conn.lock();

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS trajectories (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_name TEXT NOT NULL,
                task TEXT NOT NULL,
                task_hash TEXT NOT NULL,
                root_json TEXT NOT NULL,
                total_cost REAL,
                created_at INTEGER DEFAULT (unixepoch())
            ) STRICT;

            CREATE INDEX IF NOT EXISTS idx_traj_project ON trajectories(project_name);
            CREATE INDEX IF NOT EXISTS idx_traj_hash ON trajectories(task_hash);
            ",
        )
        .context("failed to create trajectories schema")?;

        Ok(())
    }

    /// Store a completed run trajectory.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails.
    pub fn store(
        &self,
        project_name: &str,
        task: &str,
        root: &DecompositionNode,
        total_cost: Option<f64>,
    ) -> Result<i64> {
        let _timer = ScopedTimer::new("trajectory_store");

        let task_hash = compute_task_hash(task);
        let root_json =
            serde_json::to_string(root).context("failed to serialize decomposition tree")?;

        let conn = self.storage.conn();
        let conn = conn.lock();

        conn.execute(
            "INSERT INTO trajectories (project_name, task, task_hash, root_json, total_cost)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_name, task, task_hash, root_json, total_cost],
        )
        .context("failed to insert trajectory")?;

        let id = conn.last_insert_rowid();
        tracing::info!(
            trajectory_id = id,
            project = project_name,
            task_hash,
            "trajectory stored"
        );

        Ok(id)
    }

    /// Find a trajectory by exact task hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn find_by_hash(&self, task: &str, project_name: &str) -> Result<Option<RunTrajectory>> {
        let task_hash = compute_task_hash(task);

        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, task, task_hash, root_json, total_cost, created_at
                 FROM trajectories WHERE task_hash = ?1 AND project_name = ?2 LIMIT 1",
            )
            .context("failed to prepare find_by_hash")?;

        let mut rows = stmt.query_map(params![task_hash, project_name], |row| {
            let root_json: String = row.get(4)?;
            let root: DecompositionNode = serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
                description: String::new(),
                status: "unknown".to_string(),
                children: Vec::new(),
            });

            Ok(RunTrajectory {
                id: row.get(0)?,
                project_name: row.get(1)?,
                task: row.get(2)?,
                task_hash: row.get(3)?,
                root,
                total_cost: row.get(5)?,
                created_at: row.get(6)?,
            })
        })?;

        rows.next()
            .transpose()
            .context("failed to find trajectory by hash")
    }

    /// Find similar trajectories by semantic search on task text.
    ///
    /// Falls back to LIKE matching since semantic search requires embeddings.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn find_similar(
        &self,
        task: &str,
        project_name: &str,
        options: &FindSimilarOptions,
    ) -> Result<Vec<RunTrajectory>> {
        let _timer = ScopedTimer::new("trajectory_find_similar");

        let conn = self.storage.conn();
        let conn = conn.lock();

        // Use LIKE for simple text similarity as a fallback
        let search_pattern = format!("%{task}%");

        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, task, task_hash, root_json, total_cost, created_at
                 FROM trajectories
                 WHERE project_name = ?1 AND task LIKE ?2
                 ORDER BY created_at DESC
                 LIMIT ?3",
            )
            .context("failed to prepare find_similar")?;

        #[allow(clippy::cast_possible_wrap)]
        let limit = options.top_k as i64;

        let rows: Vec<RunTrajectory> = stmt
            .query_map(params![project_name, search_pattern, limit], |row| {
                let root_json: String = row.get(4)?;
                let root: DecompositionNode = serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
                    description: String::new(),
                    status: "unknown".to_string(),
                    children: Vec::new(),
                });

                Ok(RunTrajectory {
                    id: row.get(0)?,
                    project_name: row.get(1)?,
                    task: row.get(2)?,
                    task_hash: row.get(3)?,
                    root,
                    total_cost: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        tracing::info!(
            project = project_name,
            results = rows.len(),
            "similar trajectories found"
        );

        Ok(rows)
    }

    /// Replay: extract the decomposition steps from a trajectory.
    ///
    /// Returns a flat list of step descriptions from the decomposition tree.
    ///
    /// # Errors
    ///
    /// Returns an error if the trajectory is not found.
    pub fn replay_strategy(&self, task: &str, project_name: &str) -> Result<Option<Vec<String>>> {
        let trajectory = self.find_by_hash(task, project_name)?;

        Ok(trajectory.map(|t| flatten_decomposition(&t.root)))
    }

    /// List all trajectories for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list(&self, project_name: &str, limit: i64) -> Result<Vec<RunTrajectory>> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        let mut stmt = conn
            .prepare(
                "SELECT id, project_name, task, task_hash, root_json, total_cost, created_at
                 FROM trajectories WHERE project_name = ?1
                 ORDER BY created_at DESC LIMIT ?2",
            )
            .context("failed to prepare list trajectories")?;

        let rows = stmt
            .query_map(params![project_name, limit], |row| {
                let root_json: String = row.get(4)?;
                let root: DecompositionNode = serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
                    description: String::new(),
                    status: "unknown".to_string(),
                    children: Vec::new(),
                });

                Ok(RunTrajectory {
                    id: row.get(0)?,
                    project_name: row.get(1)?,
                    task: row.get(2)?,
                    task_hash: row.get(3)?,
                    root,
                    total_cost: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        Ok(rows)
    }
}
