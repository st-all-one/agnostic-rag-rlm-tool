//! Knowledge indexing engine: file discovery, chunking, and chunk retrieval.

pub mod helpers;

pub use helpers::{compute_hash, detect_language, estimate_tokens, find_line_boundary};

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info};

use arags_storage::Storage;
use arags_storage::sqlite::chunks::NewChunk;

use arags_embedding::pipeline::discover_files;

use crate::ScopedTimer;

/// Options for indexing knowledge into a project.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    /// Maximum bytes per chunk.
    pub max_chunk_bytes: usize,
    /// Embedding model name.
    pub embedding_model: String,
    /// Embedding dimensions.
    pub embedding_dims: i64,
    /// Additional glob patterns to ignore (e.g., `["*.log", "dist/"]`).
    pub ignore_patterns: Vec<String>,
    /// Glob patterns that bypass ignore rules (e.g., `[".env", "vendor"]`).
    /// Mirrors the client `--force-include` flag.
    pub force_include: Vec<String>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            max_chunk_bytes: 1500,
            embedding_model: "all-MiniLM-L6-v2".to_string(),
            embedding_dims: 384,
            ignore_patterns: vec![],
            force_include: vec![],
        }
    }
}

/// Result of an indexing operation.
#[derive(Debug, Clone)]
pub struct IndexResult {
    /// Files processed during indexing.
    pub files_processed: u64,
    /// Chunks created during indexing.
    pub chunks_created: u64,
    /// Duration of the operation in milliseconds.
    pub duration_ms: u128,
}

/// The knowledge engine handles indexing files and retrieving content.
pub struct KnowledgeEngine {
    storage: Storage,
}

impl KnowledgeEngine {
    /// Create a new `KnowledgeEngine`.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Get the underlying storage.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Index a directory into a project buffer.
    ///
    /// Reads files, chunks them by byte size, and stores metadata in `SQLite`.
    /// Does NOT compute embeddings (that requires the embedding crate).
    ///
    /// # Errors
    ///
    /// Returns an error if directory reading or database operations fail.
    pub fn index_directory(
        &self,
        project_name: &str,
        dir_path: &Path,
        options: &IndexOptions,
    ) -> Result<IndexResult> {
        let _timer = ScopedTimer::new("knowledge_index_directory");
        let start = std::time::Instant::now();

        let buffer = self
            .storage
            .get_buffer_by_name(project_name)
            .context("failed to find project")?
            .context("project not found")?;

        let files = discover_files(
            dir_path,
            &arags_embedding::pipeline::default_index_ignores(),
            &options.ignore_patterns,
            &options.force_include,
        )
        .context("failed to discover files")?;

        let mut files_processed: u64 = 0;
        let mut chunks_created: u64 = 0;

        for path in &files {
            self.index_file(buffer.id, path, options)?;
            files_processed += 1;

            let chunks = Self::count_file_chunks(path, options.max_chunk_bytes);
            chunks_created += chunks;
        }

        self.storage
            .update_buffer_counts(
                buffer.id,
                i64::try_from(chunks_created).unwrap_or(i64::MAX),
                i64::try_from(files_processed).unwrap_or(i64::MAX),
            )
            .context("failed to update buffer counts")?;

        let duration = start.elapsed();
        info!(
            project = %project_name,
            files = files_processed,
            chunks = chunks_created,
            duration_ms = %duration.as_millis(),
            "directory indexed"
        );

        Ok(IndexResult {
            files_processed,
            chunks_created,
            duration_ms: duration.as_millis(),
        })
    }

    /// Index a single file into chunks.
    ///
    /// # Errors
    ///
    /// Returns an error if file reading or database operations fail.
    pub fn index_file(
        &self,
        buffer_id: i64,
        file_path: &Path,
        options: &IndexOptions,
    ) -> Result<Vec<i64>> {
        let start = Instant::now();
        let raw = std::fs::read(file_path)
            .with_context(|| format!("failed to read file: {}", file_path.display()))?;
        // Lossy decode: source files may contain invalid UTF-8, and the
        // byte-based chunker below can split a multi-byte character in half.
        // Neither must abort indexing — replace invalid bytes instead.
        let content = String::from_utf8_lossy(&raw).into_owned();

        let path_str = file_path.to_str().context("file path is not valid UTF-8")?;

        // Check if file has changed by computing file hash
        let file_hash = compute_hash(content.as_bytes());
        let existing_chunks = self
            .storage
            .list_chunks(buffer_id)
            .context("failed to list chunks")?;
        let file_chunks: Vec<_> = existing_chunks
            .iter()
            .filter(|c| c.file_path == path_str)
            .collect();

        // If file hash matches and chunks exist, skip re-indexing
        if !file_chunks.is_empty() {
            let stored_hash = &file_chunks[0].hash;
            if stored_hash == &file_hash {
                debug!(file = %path_str, "file unchanged, skipping");
                return Ok(file_chunks.iter().map(|c| c.id).collect());
            }
            // File changed, delete old chunks
            self.storage
                .delete_chunks_for_file(path_str)
                .context("failed to delete old chunks")?;
        }

        let bytes = content.as_bytes();
        let chunk_size = options.max_chunk_bytes;
        let mut chunk_ids = Vec::new();
        let mut offset = 0;
        let mut line_start = 1;

        while offset < bytes.len() {
            let end = (offset + chunk_size).min(bytes.len());

            // Find a line boundary near the end to avoid splitting mid-line
            let chunk_end = if end < bytes.len() {
                crate::knowledge::helpers::find_line_boundary(bytes, end)
            } else {
                bytes.len()
            };

            let chunk_bytes = &bytes[offset..chunk_end];
            // Lossy: a byte boundary can land inside a multi-byte character.
            let chunk_text = String::from_utf8_lossy(chunk_bytes).into_owned();

            let line_count = chunk_text.bytes().filter(|&b| b == b'\n').count();
            #[allow(clippy::cast_possible_wrap)]
            let line_end = line_start + line_count as i64;

            let hash = compute_hash(chunk_bytes);

            #[allow(clippy::cast_possible_wrap)]
            let chunk = NewChunk {
                buffer_id,
                file_path: path_str.to_string(),
                offset_start: offset as i64,
                offset_end: chunk_end as i64,
                line_start,
                line_end,
                hash,
                language: detect_language(file_path),
                chunk_type: None,
                token_count: Some(estimate_tokens(&chunk_text)),
            };

            let chunk_id = self
                .storage
                .insert_chunk(&chunk)
                .context("failed to insert chunk")?;

            self.storage
                .insert_chunk_content(chunk_id, &chunk_text)
                .context("failed to insert chunk content")?;

            // Extract and store entities for entity search tier
            let entities = arags_storage::Storage::extract_entities(&chunk_text, path_str);
            if !entities.is_empty() {
                self.storage
                    .insert_chunk_entities(chunk_id, &entities)
                    .context("failed to insert chunk entities")?;
            }

            chunk_ids.push(chunk_id);

            offset = chunk_end;
            line_start = line_end + 1;
        }

        debug!(
            file = %path_str,
            chunks = chunk_ids.len(),
            duration_ms = %start.elapsed().as_millis(),
            "file indexed"
        );

        Ok(chunk_ids)
    }

    /// Retrieve chunk content by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn get_chunk_content(&self, chunk_id: i64) -> Result<Option<String>> {
        self.storage
            .get_chunk_content(chunk_id)
            .context("failed to get chunk content")
    }

    /// List all chunks for a project.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn list_chunks(&self, buffer_id: i64) -> Result<Vec<arags_storage::sqlite::chunks::Chunk>> {
        self.storage
            .list_chunks(buffer_id)
            .context("failed to list chunks")
    }

    fn count_file_chunks(path: &Path, max_chunk_bytes: usize) -> u64 {
        std::fs::read(path).map_or(0, |bytes| {
            let len = bytes.len();
            if len == 0 {
                1
            } else {
                ((len - 1) / max_chunk_bytes + 1) as u64
            }
        })
    }
}
