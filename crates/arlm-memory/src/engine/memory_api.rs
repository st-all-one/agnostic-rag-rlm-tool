//! Core [`MemoryEngine`] API surface: lifecycle, accessors, and projections.
//!
//! Project/session/trajectory/persistence convenience methods plus the
//! `arlm-core` [`MemoryProvider`](arlm_core::memory::MemoryProvider) integration
//! (context injection + trajectory persistence).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use arlm_core::memory::MemoryProvider;
use arlm_core::types::{RlmNode, RlmRunResult, StartRunInput};
use arlm_storage::Storage;

use super::{MemoryEngine, SearchOptions};
use crate::ScopedTimer;
use crate::knowledge::KnowledgeEngine;
use crate::persist::PersistEngine;
use crate::persist::SearchPersistOptions;
use crate::project::ProjectManager;
use crate::session::SessionManager;
use crate::trajectory::{DecompositionNode, FindSimilarOptions, RunTrajectory, TrajectoryEngine};

impl MemoryEngine {
    /// Create a new `MemoryEngine` backed by the given storage path.
    ///
    /// Ensures all required schemas exist.
    ///
    /// # Errors
    ///
    /// Returns an error if schema creation or storage opening fails.
    pub fn open(storage_path: &Path) -> Result<Self> {
        let _timer = ScopedTimer::new("memory_engine_open");
        let storage = Storage::open(storage_path).context("failed to open storage")?;

        let projects = ProjectManager::new(storage.clone());
        let knowledge = KnowledgeEngine::new(storage.clone());
        let sessions = SessionManager::new(storage.clone())?;
        let trajectories = TrajectoryEngine::new(storage.clone())?;
        let persist = PersistEngine::new(storage_path)?;

        tracing::info!(path = %storage_path.display(), "MemoryEngine opened");

        Ok(Self {
            storage,
            projects,
            knowledge,
            sessions,
            trajectories,
            persist,
            project_path: storage_path.to_path_buf(),
        })
    }

    /// Create a new `MemoryEngine` with an existing storage instance.
    ///
    /// # Errors
    ///
    /// Returns an error if session or trajectory schema creation fails.
    pub fn new(storage: Storage, project_path: PathBuf) -> Result<Self> {
        let projects = ProjectManager::new(storage.clone());
        let knowledge = KnowledgeEngine::new(storage.clone());
        let sessions = SessionManager::new(storage.clone())?;
        let trajectories = TrajectoryEngine::new(storage.clone())?;
        let persist = PersistEngine::new(&project_path)?;

        Ok(Self {
            storage,
            projects,
            knowledge,
            sessions,
            trajectories,
            persist,
            project_path,
        })
    }

    /// Get a reference to the underlying storage.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Get a reference to the project manager.
    #[must_use]
    pub fn projects(&self) -> &ProjectManager {
        &self.projects
    }

    /// Get a reference to the knowledge engine.
    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeEngine {
        &self.knowledge
    }

    /// Get a reference to the session manager.
    #[must_use]
    pub fn sessions(&self) -> &SessionManager {
        &self.sessions
    }

    /// Get a reference to the trajectory engine.
    #[must_use]
    pub fn trajectories(&self) -> &TrajectoryEngine {
        &self.trajectories
    }

    /// Get a reference to the persist engine.
    #[must_use]
    pub fn persist(&self) -> &PersistEngine {
        &self.persist
    }

    // === PROJECT LIFECYCLE ===

    /// Create a new project.
    ///
    /// # Errors
    ///
    /// Returns an error if the project already exists or storage fails.
    pub fn create_project(&self, name: &str, path: &Path) -> Result<crate::project::ProjectInfo> {
        self.projects.create(&crate::project::CreateProjectOptions {
            name: name.to_string(),
            path: path.to_path_buf(),
        })
    }

    /// List all projects.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn list_projects(&self) -> Result<Vec<crate::project::ProjectInfo>> {
        self.projects.list()
    }

    /// Get a project by name.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn get_project(&self, name: &str) -> Result<Option<crate::project::ProjectInfo>> {
        self.projects.get(name)
    }

    // === SESSIONS ===

    /// Create a new multi-turn session.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn create_session(&self, project_name: &str, title: &str) -> Result<String> {
        self.sessions.create(project_name, title)
    }

    /// Add a context version to a session.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn add_session_context(&self, session_id: &str, payload: &str) -> Result<u32> {
        self.sessions.add_context(session_id, payload)
    }

    /// Record a query in the session history.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn record_session_query(
        &self,
        session_id: &str,
        query: &str,
        result: Option<&str>,
    ) -> Result<i64> {
        self.sessions.record_query(session_id, query, result)
    }

    // === TRAJECTORIES ===

    /// Store a run trajectory for future reuse.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn store_trajectory(
        &self,
        project_name: &str,
        task: &str,
        root: &DecompositionNode,
        total_cost: Option<f64>,
    ) -> Result<i64> {
        self.trajectories
            .store(project_name, task, root, total_cost)
    }

    /// Find similar past trajectories by task hash.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn find_similar_trajectories(
        &self,
        task: &str,
        project_name: &str,
    ) -> Result<Vec<RunTrajectory>> {
        self.trajectories
            .find_similar(task, project_name, &FindSimilarOptions::default())
    }

    // === PERSISTENCE ===

    /// Persist search results as a wiki page.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence fails.
    pub fn persist_search(
        &self,
        project_name: &str,
        query: &str,
        body: &str,
        tier: &str,
    ) -> Result<String> {
        let result = self.persist.persist_search(&SearchPersistOptions {
            query: query.to_string(),
            tier: tier.to_string(),
            project: project_name.to_string(),
            entities: Vec::new(),
            tags: Vec::new(),
            body: body.to_string(),
        })?;
        Ok(result.path.to_string_lossy().into_owned())
    }
}

impl MemoryProvider for MemoryEngine {
    /// Retrieve relevant memory context strings for a given task.
    ///
    /// Runs a BM25 search over the indexed knowledge and returns the matching
    /// chunk contents as context for the solver.
    fn context(&self, task: &str) -> Result<Vec<String>, String> {
        let _timer = ScopedTimer::new("memory_context");
        let results = self
            .search(task, &SearchOptions::default())
            .map_err(|e| e.to_string())?;
        let ctx: Vec<String> = results.into_iter().map(|r| r.content).collect();
        tracing::info!(task, results = ctx.len(), "memory context retrieved");
        Ok(ctx)
    }

    /// Persist a completed run's trajectory.
    ///
    /// Converts the core decision tree into the memory decomposition format and
    /// stores it under the run's project.
    fn save_trajectory(&self, input: &StartRunInput, result: &RlmRunResult) -> Result<(), String> {
        let _timer = ScopedTimer::new("memory_save_trajectory");
        let root = decompose_from_node(&result.root);
        let total_cost = result.root.total_usage().cost_usd;
        self.store_trajectory(&input.project, &input.task, &root, Some(total_cost))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Convert a core decision tree node into the memory decomposition format.
#[must_use]
pub(crate) fn decompose_from_node(node: &RlmNode) -> DecompositionNode {
    DecompositionNode {
        description: node.task.clone(),
        status: node.status.to_string(),
        children: node.children.iter().map(decompose_from_node).collect(),
    }
}
