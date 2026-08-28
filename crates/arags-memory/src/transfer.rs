use std::time::Instant;

use anyhow::{Context, Result};

use arags_storage::Storage;
use tracing::{debug, info};

use crate::ScopedTimer;

/// Options for knowledge transfer between projects.
#[derive(Debug, Clone)]
pub struct TransferOptions {
    /// Only transfer chunks matching these languages.
    pub languages: Vec<String>,
    /// Maximum number of chunks to transfer.
    pub max_chunks: usize,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            max_chunks: 1000,
        }
    }
}

/// Result of a transfer operation.
#[derive(Debug, Clone)]
pub struct TransferResult {
    pub chunks_transferred: u64,
}

/// Handles knowledge transfer between projects.
pub struct TransferEngine {
    storage: Storage,
}

impl TransferEngine {
    /// Create a new `TransferEngine`.
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    /// Transfer chunks from one project to another.
    ///
    /// Copies chunk metadata and content from `from_buffer_id` to `to_buffer_id`.
    /// Optionally filters by language and limits the count.
    ///
    /// # Errors
    ///
    /// Returns an error if source or target project not found, or DB operations fail.
    pub fn transfer(
        &self,
        from_buffer_id: i64,
        to_buffer_id: i64,
        options: &TransferOptions,
    ) -> Result<TransferResult> {
        let _timer = ScopedTimer::new("knowledge_transfer");

        let verify_start = Instant::now();
        // Verify both projects exist
        let _source = self
            .storage
            .get_buffer(from_buffer_id)
            .context("failed to get source project")?
            .context("source project not found")?;

        let _target = self
            .storage
            .get_buffer(to_buffer_id)
            .context("failed to get target project")?
            .context("target project not found")?;
        debug!(
            from = from_buffer_id,
            to = to_buffer_id,
            duration_ms = %verify_start.elapsed().as_millis(),
            "transfer project verification complete"
        );

        let list_start = Instant::now();
        let chunks = self
            .storage
            .list_chunks(from_buffer_id)
            .context("failed to list source chunks")?;

        let filtered: Vec<_> = if options.languages.is_empty() {
            chunks
        } else {
            chunks
                .into_iter()
                .filter(|c| {
                    c.language
                        .as_ref()
                        .is_some_and(|l| options.languages.contains(l))
                })
                .collect()
        };

        let limited: Vec<_> = filtered.into_iter().take(options.max_chunks).collect();
        debug!(
            from = from_buffer_id,
            candidate_chunks = limited.len(),
            duration_ms = %list_start.elapsed().as_millis(),
            "transfer chunk selection complete"
        );

        let copy_start = Instant::now();
        let mut transferred: u64 = 0;

        for chunk in &limited {
            // Copy chunk metadata to target
            let new_chunk = arags_storage::sqlite::chunks::NewChunk {
                buffer_id: to_buffer_id,
                file_path: chunk.file_path.clone(),
                offset_start: chunk.offset_start,
                offset_end: chunk.offset_end,
                line_start: chunk.line_start,
                line_end: chunk.line_end,
                hash: chunk.hash.clone(),
                language: chunk.language.clone(),
                chunk_type: chunk.chunk_type.clone(),
                token_count: chunk.token_count,
            };

            let new_id = self
                .storage
                .insert_chunk(&new_chunk)
                .context("failed to insert transferred chunk")?;

            // Copy content if available
            if let Ok(Some(content)) = self.storage.get_chunk_content(chunk.id) {
                self.storage
                    .insert_chunk_content(new_id, &content)
                    .context("failed to transfer chunk content")?;
            }

            transferred += 1;
        }
        debug!(
            from = from_buffer_id,
            to = to_buffer_id,
            copied = transferred,
            duration_ms = %copy_start.elapsed().as_millis(),
            "transfer chunk copy complete"
        );

        // Update target counts
        let counts_start = Instant::now();
        let target_chunks = self
            .storage
            .count_chunks(to_buffer_id)
            .context("failed to count target chunks")?;

        self.storage
            .update_buffer_counts(to_buffer_id, target_chunks, 0)
            .context("failed to update target counts")?;
        debug!(
            to = to_buffer_id,
            target_chunks,
            duration_ms = %counts_start.elapsed().as_millis(),
            "transfer count update complete"
        );

        info!(
            from = from_buffer_id,
            to = to_buffer_id,
            transferred,
            "knowledge transferred"
        );

        Ok(TransferResult {
            chunks_transferred: transferred,
        })
    }

    /// Identify common patterns between two projects.
    ///
    /// Returns pattern names that exist in both projects.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn common_patterns(&self, from_buffer_id: i64, to_buffer_id: i64) -> Result<Vec<String>> {
        let _timer = ScopedTimer::new("knowledge_common_patterns");

        let source_patterns = self
            .storage
            .get_patterns(Some(from_buffer_id))
            .context("failed to get source patterns")?;

        let target_patterns = self
            .storage
            .get_patterns(Some(to_buffer_id))
            .context("failed to get target patterns")?;

        let target_names: std::collections::HashSet<String> =
            target_patterns.into_iter().map(|p| p.name).collect();

        let common: Vec<String> = source_patterns
            .into_iter()
            .filter(|p| target_names.contains(&p.name))
            .map(|p| p.name)
            .collect();

        info!(
            from = from_buffer_id,
            to = to_buffer_id,
            common_count = common.len(),
            "common patterns identified"
        );

        Ok(common)
    }
}
