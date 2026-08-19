use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use arlm_storage::Storage;

use crate::ScopedTimer;

/// A complete run trajectory capturing the decomposition and results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTrajectory {
    pub id: i64,
    pub project_name: String,
    pub task: String,
    pub task_hash: String,
    pub root: DecompositionNode,
    pub total_cost: Option<f64>,
    pub created_at: i64,
}

/// A node in the decomposition tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompositionNode {
    pub description: String,
    pub status: String,
    pub children: Vec<DecompositionNode>,
}

/// Options for finding similar runs.
#[derive(Debug, Clone)]
pub struct FindSimilarOptions {
    /// Similarity score threshold (0.0 - 1.0).
    pub min_score: f32,
    /// Maximum results to return.
    pub top_k: usize,
}

impl Default for FindSimilarOptions {
    fn default() -> Self {
        Self {
            min_score: 0.7,
            top_k: 5,
        }
    }
}

/// The trajectory engine stores and retrieves run strategies.
pub struct TrajectoryEngine {
    storage: Storage,
}

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
    pub fn find_by_hash(
        &self,
        task: &str,
        project_name: &str,
    ) -> Result<Option<RunTrajectory>> {
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
            let root: DecompositionNode =
                serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
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

        rows.next().transpose().context("failed to find trajectory by hash")
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
                let root: DecompositionNode =
                    serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
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
    pub fn replay_strategy(
        &self,
        task: &str,
        project_name: &str,
    ) -> Result<Option<Vec<String>>> {
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
                let root: DecompositionNode =
                    serde_json::from_str(&root_json).unwrap_or(DecompositionNode {
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

fn compute_task_hash(task: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(task.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

fn flatten_decomposition(node: &DecompositionNode) -> Vec<String> {
    let mut steps = Vec::new();
    if !node.description.is_empty() {
        steps.push(node.description.clone());
    }
    for child in &node.children {
        steps.extend(flatten_decomposition(child));
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TrajectoryEngine, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        (TrajectoryEngine::new(storage).unwrap(), tmp)
    }

    fn make_root() -> DecompositionNode {
        DecompositionNode {
            description: "root task".to_string(),
            status: "completed".to_string(),
            children: vec![
                DecompositionNode {
                    description: "subtask 1".to_string(),
                    status: "completed".to_string(),
                    children: Vec::new(),
                },
                DecompositionNode {
                    description: "subtask 2".to_string(),
                    status: "completed".to_string(),
                    children: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn test_store_and_find() {
        let (eng, _tmp) = setup();
        let root = make_root();

        let id = eng
            .store("my-project", "fix the bug", &root, Some(0.5))
            .unwrap();
        assert!(id > 0);

        let found = eng.find_by_hash("fix the bug", "my-project").unwrap();
        assert!(found.is_some());

        let t = found.unwrap();
        assert_eq!(t.task, "fix the bug");
        assert_eq!(t.root.children.len(), 2);
    }

    #[test]
    fn test_find_by_hash_not_found() {
        let (eng, _tmp) = setup();
        let result = eng.find_by_hash("nonexistent", "proj").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_find_similar() {
        let (eng, _tmp) = setup();
        let root = make_root();

        eng.store("proj", "fix login bug", &root, None).unwrap();
        eng.store("proj", "fix auth bug", &root, None).unwrap();
        eng.store("proj", "add tests", &root, None).unwrap();

        let opts = FindSimilarOptions::default();
        let similar = eng.find_similar("fix", "proj", &opts).unwrap();
        assert_eq!(similar.len(), 2);
    }

    #[test]
    fn test_replay_strategy() {
        let (eng, _tmp) = setup();
        let root = make_root();

        eng.store("proj", "deploy app", &root, None).unwrap();

        let steps = eng.replay_strategy("deploy app", "proj").unwrap();
        assert!(steps.is_some());

        let steps = steps.unwrap();
        assert_eq!(steps.len(), 3); // root + 2 children
        assert!(steps.contains(&"root task".to_string()));
        assert!(steps.contains(&"subtask 1".to_string()));
    }

    #[test]
    fn test_list() {
        let (eng, _tmp) = setup();
        let root = DecompositionNode {
            description: "t".to_string(),
            status: "done".to_string(),
            children: Vec::new(),
        };

        eng.store("proj", "task1", &root, None).unwrap();
        eng.store("proj", "task2", &root, None).unwrap();

        let list = eng.list("proj", 10).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_flatten_decomposition() {
        let root = make_root();
        let steps = flatten_decomposition(&root);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], "root task");
        assert_eq!(steps[1], "subtask 1");
        assert_eq!(steps[2], "subtask 2");
    }

    #[test]
    fn test_compute_task_hash_deterministic() {
        let h1 = compute_task_hash("test task");
        let h2 = compute_task_hash("test task");
        let h3 = compute_task_hash("other task");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }
}
