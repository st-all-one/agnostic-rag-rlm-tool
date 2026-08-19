use anyhow::{Context, Result};

use arlm_storage::Storage;

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
        let mut transferred: u64 = 0;

        for chunk in &limited {
            // Copy chunk metadata to target
            let new_chunk = arlm_storage::sqlite::chunks::NewChunk {
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

        // Update target counts
        let target_chunks = self
            .storage
            .count_chunks(to_buffer_id)
            .context("failed to count target chunks")?;

        self.storage
            .update_buffer_counts(to_buffer_id, target_chunks, 0)
            .context("failed to update target counts")?;

        tracing::info!(
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

        tracing::info!(
            from = from_buffer_id,
            to = to_buffer_id,
            common_count = common.len(),
            "common patterns identified"
        );

        Ok(common)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlm_storage::sqlite::buffers::NewBuffer;
    use arlm_storage::sqlite::chunks::NewChunk;
    use tempfile::TempDir;

    fn setup() -> (TransferEngine, Storage, TempDir) {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        let engine = TransferEngine::new(storage.clone());
        (engine, storage, tmp)
    }

    fn create_buffer(storage: &Storage, name: &str) -> i64 {
        storage
            .insert_buffer(&NewBuffer {
                name: name.to_string(),
                path: format!("/tmp/{name}"),
            })
            .unwrap()
    }

    #[test]
    fn test_transfer_chunks() {
        let (engine, storage, _tmp) = setup();
        let from_id = create_buffer(&storage, "source");
        let to_id = create_buffer(&storage, "target");

        // Insert chunks into source
        storage
            .insert_chunk(&NewChunk {
                buffer_id: from_id,
                file_path: "a.rs".to_string(),
                offset_start: 0,
                offset_end: 50,
                line_start: 1,
                line_end: 5,
                hash: vec![1],
                language: Some("rust".to_string()),
                chunk_type: None,
                token_count: Some(10),
            })
            .unwrap();

        let opts = TransferOptions::default();
        let result = engine.transfer(from_id, to_id, &opts).unwrap();
        assert_eq!(result.chunks_transferred, 1);

        let target_chunks = storage.count_chunks(to_id).unwrap();
        assert_eq!(target_chunks, 1);
    }

    #[test]
    fn test_transfer_with_language_filter() {
        let (engine, storage, _tmp) = setup();
        let from_id = create_buffer(&storage, "source");
        let to_id = create_buffer(&storage, "target");

        storage
            .insert_chunk(&NewChunk {
                buffer_id: from_id,
                file_path: "a.rs".to_string(),
                offset_start: 0,
                offset_end: 50,
                line_start: 1,
                line_end: 5,
                hash: vec![1],
                language: Some("rust".to_string()),
                chunk_type: None,
                token_count: None,
            })
            .unwrap();

        storage
            .insert_chunk(&NewChunk {
                buffer_id: from_id,
                file_path: "b.py".to_string(),
                offset_start: 0,
                offset_end: 50,
                line_start: 1,
                line_end: 5,
                hash: vec![2],
                language: Some("python".to_string()),
                chunk_type: None,
                token_count: None,
            })
            .unwrap();

        let opts = TransferOptions {
            languages: vec!["rust".to_string()],
            max_chunks: 100,
        };

        let result = engine.transfer(from_id, to_id, &opts).unwrap();
        assert_eq!(result.chunks_transferred, 1);
    }

    #[test]
    fn test_transfer_max_chunks_limit() {
        let (engine, storage, _tmp) = setup();
        let from_id = create_buffer(&storage, "source");
        let to_id = create_buffer(&storage, "target");

        for i in 0..10 {
            storage
                .insert_chunk(&NewChunk {
                    buffer_id: from_id,
                    file_path: format!("f{i}.rs"),
                    offset_start: 0,
                    offset_end: 50,
                    line_start: 1,
                    line_end: 5,
                    hash: vec![i as u8],
                    language: None,
                    chunk_type: None,
                    token_count: None,
                })
                .unwrap();
        }

        let opts = TransferOptions {
            languages: Vec::new(),
            max_chunks: 3,
        };

        let result = engine.transfer(from_id, to_id, &opts).unwrap();
        assert_eq!(result.chunks_transferred, 3);
    }
}
