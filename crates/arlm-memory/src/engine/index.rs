//! Project indexing operations for [`MemoryEngine`](crate::engine::MemoryEngine).

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use arlm_embedding::embedder::Embedder;
use arlm_embedding::pipeline::{discover_files, IngestionPipeline};

use crate::engine::{IndexProjectOptions, IndexProjectResult, MemoryEngine};
use crate::knowledge::IndexOptions;
use crate::ScopedTimer;

impl MemoryEngine {
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
            self.projects.create(&crate::project::CreateProjectOptions {
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
                ignore_patterns: options.ignore_patterns.clone(),
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
            self.projects.create(&crate::project::CreateProjectOptions {
                name: options.project_name.clone(),
                path: options.dir_path.clone(),
            })?;
        }

        // Discover files
        let files = discover_files(&options.dir_path, &options.ignore_patterns)
            .context("failed to discover files")?;
        let total_files = files.len();

        // Run ingestion pipeline
        let cache_path = std::env::temp_dir().join("arlm_embedding_cache.db");
        let cache = arlm_embedding::embedder::cache::EmbeddingCache::open(
            cache_path.to_str().unwrap_or(":memory:"),
            1024,
        )
        .ok();
        let pipeline = IngestionPipeline::new(embedder, cache);
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
}
