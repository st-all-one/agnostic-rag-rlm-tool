//! Unified orchestrator for the memory subsystem.
//!
//! Coordinates project management, knowledge indexing, session tracking,
//! trajectory storage, and persistence into a single coherent API.

pub mod index;
pub mod search;

use std::path::Path;

use anyhow::Result;
use arags_storage::Storage;

use crate::knowledge::KnowledgeEngine;
use crate::project::ProjectManager;

/// Unified orchestrator for the memory subsystem.
///
/// Coordinates project management, knowledge indexing, and persistence into a
/// single coherent API. (Session/trajectory tracking were removed in plan 019.)
pub struct MemoryEngine {
    pub(crate) storage: Storage,
    pub(crate) projects: ProjectManager,
    pub(crate) knowledge: KnowledgeEngine,
    #[allow(dead_code)]
    pub(crate) project_path: std::path::PathBuf,
}

impl MemoryEngine {
    /// Open the shared memory storage engine for the given project path.
    ///
    /// The same storage handle backs project management, knowledge indexing,
    /// FTS5 search, the QA cache and history.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying storage cannot be opened.
    pub fn open(project_path: &Path) -> Result<Self> {
        let storage = Storage::open(project_path)?;
        let projects = ProjectManager::new(storage.clone());
        let knowledge = KnowledgeEngine::new(storage.clone());
        Ok(Self {
            storage,
            projects,
            knowledge,
            project_path: project_path.to_path_buf(),
        })
    }
}

/// Options for indexing a project directory.
#[derive(Debug, Clone)]
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
    /// Additional glob patterns to ignore.
    pub ignore_patterns: Vec<String>,
    /// Glob patterns that bypass ignore rules (mirrors `--force-include`).
    pub force_include: Vec<String>,
}

impl Default for IndexProjectOptions {
    fn default() -> Self {
        Self {
            project_name: String::new(),
            dir_path: std::path::PathBuf::new(),
            max_chunk_bytes: 1500,
            embedding_model: "all-MiniLM-L6-v2".to_string(),
            embedding_dims: 384,
            ignore_patterns: Vec::new(),
            force_include: Vec::new(),
        }
    }
}

/// Result of indexing a project.
#[derive(Debug, Clone)]
pub struct IndexProjectResult {
    /// Files processed during indexing.
    pub files_processed: u64,
    /// Chunks created during indexing.
    pub chunks_created: u64,
    /// Duration of the operation in milliseconds.
    pub duration_ms: u128,
}

/// Options for searching across the memory.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// Optional project name to scope the search.
    pub project_name: Option<String>,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Search tier to use (e.g. "entity", "fts").
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
    /// Chunk row id.
    pub chunk_id: i64,
    /// Source file path.
    pub file_path: String,
    /// Chunk content.
    pub content: String,
    /// Relevance score.
    pub score: f32,
}
