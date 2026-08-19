use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use arlm_embedding::embedder::Embedder;
use arlm_embedding::pipeline::{discover_files, IngestionPipeline};
use arlm_storage::Storage;

use crate::knowledge::{IndexOptions, KnowledgeEngine};
use crate::persist::{PersistEngine, SearchPersistOptions};
use crate::project::{CreateProjectOptions, ProjectInfo, ProjectManager};
use crate::session::SessionManager;
use crate::trajectory::{FindSimilarOptions, TrajectoryEngine};
use crate::ScopedTimer;

/// Unified orchestrator for the memory subsystem.
///
/// Coordinates project management, knowledge indexing, session tracking,
/// trajectory storage, and persistence into a single coherent API.
pub struct MemoryEngine {
    storage: Storage,
    projects: ProjectManager,
    knowledge: KnowledgeEngine,
    sessions: SessionManager,
    trajectories: TrajectoryEngine,
    persist: PersistEngine,
    #[allow(dead_code)]
    project_path: std::path::PathBuf,
}

/// Options for indexing a project directory.
pub struct IndexProjectOptions {
    /// Project name (must be unique).
    pub project_name: String,
    /// Root directory to index.
    pub dir_path: std::path::PathBuf,
    /// Maximum bytes per chunk.
    pub max_chunk_bytes: usize,
    /// Embedding model name.
    pub embedding_model: String,
    /// Embedding dimensions.
    pub embedding_dims: i64,
}

impl Default for IndexProjectOptions {
    fn default() -> Self {
        Self {
            project_name: String::new(),
            dir_path: std::path::PathBuf::new(),
            max_chunk_bytes: 1500,
            embedding_model: "bge-m3".to_string(),
            embedding_dims: 1024,
        }
    }
}

/// Result of indexing a project.
#[derive(Debug, Clone)]
pub struct IndexProjectResult {
    pub files_processed: u64,
    pub chunks_created: u64,
    pub duration_ms: u128,
}

/// Options for searching across the memory.
pub struct SearchOptions {
    pub project_name: Option<String>,
    pub limit: usize,
    pub tier: String,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            project_name: None,
            limit: 10,
            tier: "entity".to_string(),
        }
    }
}

/// A search result from the memory engine.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk_id: i64,
    pub file_path: String,
    pub content: String,
    pub score: f32,
}

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

        info!(path = %storage_path.display(), "MemoryEngine opened");

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
    pub fn create_project(&self, name: &str, path: &Path) -> Result<ProjectInfo> {
        self.projects.create(&CreateProjectOptions {
            name: name.to_string(),
            path: path.to_path_buf(),
        })
    }

    /// List all projects.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        self.projects.list()
    }

    /// Get a project by name.
    ///
    /// # Errors
    ///
    /// Returns an error if storage fails.
    pub fn get_project(&self, name: &str) -> Result<Option<ProjectInfo>> {
        self.projects.get(name)
    }

    // === KNOWLEDGE INDEXING ===

    /// Index a project directory: discover files, chunk, and store metadata.
    ///
    /// Does NOT compute embeddings — that requires the embedding pipeline separately.
    ///
    /// # Errors
    ///
    /// Returns an error if directory reading, chunking, or storage fails.
    pub fn index_project(&self, options: &IndexProjectOptions) -> Result<IndexProjectResult> {
        let _timer = ScopedTimer::new("memory_index_project");

        // Ensure project exists
        if self.projects.get(&options.project_name)?.is_none() {
            self.projects.create(&CreateProjectOptions {
                name: options.project_name.clone(),
                path: options.dir_path.clone(),
            })?;
        }

        let index_result = self.knowledge.index_directory(
            &options.project_name,
            &options.dir_path,
            &IndexOptions {
                max_chunk_bytes: options.max_chunk_bytes,
                embedding_model: options.embedding_model.clone(),
                embedding_dims: options.embedding_dims,
            },
        )?;

        info!(
            project = options.project_name,
            files = index_result.files_processed,
            chunks = index_result.chunks_created,
            duration_ms = index_result.duration_ms,
            "project indexed"
        );

        Ok(IndexProjectResult {
            files_processed: index_result.files_processed,
            chunks_created: index_result.chunks_created,
            duration_ms: index_result.duration_ms,
        })
    }

    /// Index a project with embeddings using the provided embedder.
    ///
    /// This performs full ingestion: file discovery → chunking → embedding → storage.
    ///
    /// # Errors
    ///
    /// Returns an error if any step fails.
    pub fn index_project_with_embeddings(
        &self,
        options: &IndexProjectOptions,
        embedder: Arc<dyn Embedder>,
    ) -> Result<IndexProjectResult> {
        let _timer = ScopedTimer::new("memory_index_project_with_embeddings");
        let start = std::time::Instant::now();

        // Ensure project exists
        if self.projects.get(&options.project_name)?.is_none() {
            self.projects.create(&CreateProjectOptions {
                name: options.project_name.clone(),
                path: options.dir_path.clone(),
            })?;
        }

        // Discover files
        let files =
            discover_files(&options.dir_path).context("failed to discover files")?;
        let total_files = files.len();

        // Run ingestion pipeline
        let pipeline = IngestionPipeline::new(embedder, None);
        let ingest_options = arlm_embedding::pipeline::IngestOptions {
            max_tokens: 512,
            overlap_tokens: 64,
            batch_size: 64,
            compress: true,
        };
        let result = pipeline
            .ingest(&files, &ingest_options)
            .context("ingestion pipeline failed")?;

        let duration_ms = start.elapsed().as_millis();

        info!(
            project = options.project_name,
            files = total_files,
            chunks = result.total_chunks,
            embedded = result.total_embedded,
            duration_ms,
            "project indexed with embeddings"
        );

        Ok(IndexProjectResult {
            files_processed: total_files as u64,
            chunks_created: result.total_chunks as u64,
            duration_ms,
        })
    }

    // === SEARCH ===

    /// Search across indexed knowledge using FTS5 BM25.
    ///
    /// # Errors
    ///
    /// Returns an error if the search query fails.
    pub fn search(&self, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>> {
        let _timer = ScopedTimer::new("memory_search");

        let conn = self.storage.conn();
        let conn = conn.lock();

        let limit = options.limit as i64;
        let sql = "SELECT c.id, c.file_path, c.content, bm25(chunks_fts) AS rank
                   FROM chunks_fts
                   JOIN chunks c ON c.rowid = chunks_fts.rowid
                   WHERE chunks_fts.content MATCH ?1
                   ORDER BY rank
                   LIMIT ?2";

        let mut stmt = conn
            .prepare(sql)
            .context("failed to prepare FTS search")?;

        let rows: Vec<SearchResult> = stmt
            .query_map(rusqlite::params![query, limit], |row| {
                Ok(SearchResult {
                    chunk_id: row.get(0)?,
                    file_path: row.get(1)?,
                    content: row.get(2)?,
                    score: row.get::<_, f32>(3)?.abs(),
                })
            })?
            .filter_map(std::result::Result::ok)
            .collect();

        info!(query, results = rows.len(), "search completed");

        Ok(rows)
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
        root: &crate::trajectory::DecompositionNode,
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
    ) -> Result<Vec<crate::trajectory::RunTrajectory>> {
        self.trajectories.find_similar(
            task,
            project_name,
            &FindSimilarOptions::default(),
        )
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (MemoryEngine, TempDir) {
        let tmp = TempDir::new().unwrap();
        let engine = MemoryEngine::open(tmp.path()).unwrap();
        (engine, tmp)
    }

    #[test]
    fn test_create_project() {
        let (engine, _tmp) = setup();
        let info = engine
            .create_project("test-proj", Path::new("/tmp/test"))
            .unwrap();
        assert_eq!(info.name, "test-proj");
    }

    #[test]
    fn test_list_projects_empty() {
        let (engine, _tmp) = setup();
        let projects = engine.list_projects().unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn test_get_project() {
        let (engine, _tmp) = setup();
        engine
            .create_project("my-proj", Path::new("/tmp"))
            .unwrap();
        let proj = engine.get_project("my-proj").unwrap();
        assert!(proj.is_some());
        assert_eq!(proj.unwrap().name, "my-proj");
    }

    #[test]
    fn test_create_session() {
        let (engine, _tmp) = setup();
        let id = engine.create_session("proj", "Analysis").unwrap();
        assert!(id.starts_with("s_"));
    }

    #[test]
    fn test_session_context() {
        let (engine, _tmp) = setup();
        let id = engine.create_session("proj", "title").unwrap();
        let v = engine.add_session_context(&id, "context data").unwrap();
        assert_eq!(v, 1);
    }

    #[test]
    fn test_store_and_find_trajectory() {
        let (engine, _tmp) = setup();
        let root = crate::trajectory::DecompositionNode {
            description: "root task".to_string(),
            status: "completed".to_string(),
            children: vec![],
        };
        let id = engine
            .store_trajectory("proj", "test task", &root, Some(0.05))
            .unwrap();
        assert!(id > 0);

        let similar = engine.find_similar_trajectories("test task", "proj").unwrap();
        assert!(!similar.is_empty());
    }
}
