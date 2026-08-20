pub mod cost;
pub mod progress;
pub mod strategy;

use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role};
use arlm_storage::Storage;

use self::cost::estimate_cost;
use self::progress::ProgressTracker;
use self::strategy::{build_summary_prompt, parse_summary_response, RawChunk};

/// Summarization engine that produces hierarchical summaries.
pub struct Summarizer {
    storage: Storage,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    tracker: ProgressTracker,
}

impl Summarizer {
    /// Create a new summarizer.
    pub fn new(storage: Storage, llm: Arc<dyn LlmBackend + Send + Sync>) -> Self {
        Self {
            storage,
            llm,
            tracker: ProgressTracker::new(),
        }
    }

    /// Get the progress tracker.
    pub fn tracker(&self) -> &ProgressTracker {
        &self.tracker
    }

    /// Summarize all chunks for a project.
    ///
    /// # Arguments
    ///
    /// * `buffer_id` - The project/buffer ID
    /// * `max_concurrent` - Maximum concurrent LLM calls
    ///
    /// # Errors
    ///
    /// Returns an error if summarization fails.
    pub async fn summarize_project(&self, buffer_id: i64, max_concurrent: u32) -> Result<SummaryResult> {
        let conn = self.storage.conn();
        let conn = conn.lock();

        // Get all chunks for this buffer
        let mut stmt = conn.prepare(
            "SELECT id, content, file_path FROM chunks WHERE buffer_id = ?1 ORDER BY file_path, start_line"
        )?;

        let chunks: Vec<RawChunk> = stmt
            .query_map([buffer_id], |row| {
                Ok(RawChunk {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    file_path: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        if chunks.is_empty() {
            return Ok(SummaryResult {
                file_summaries: 0,
                module_summaries: 0,
                project_summaries: 0,
                total_summarized: 0,
            });
        }

        // Group chunks by file
        let mut file_groups: std::collections::HashMap<String, Vec<&RawChunk>> = std::collections::HashMap::new();
        for chunk in &chunks {
            file_groups
                .entry(chunk.file_path.clone())
                .or_default()
                .push(chunk);
        }

        let total_files = file_groups.len() as u32;
        self.tracker.start(total_files);

        let mut file_summaries = 0u32;
        let mut module_groups: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

        // Per-file summarization
        for (file_path, file_chunks) in &file_groups {
            let prompt = build_summary_prompt(file_chunks, "file")?;

            // Call LLM to generate summary
            let summary = self.call_llm(&prompt).await?;

            // Store the summary
            let source_chunk_ids: Vec<i64> = file_chunks.iter().map(|c| c.id).collect();
            let source_hash = compute_hash(&summary);

            conn.execute(
                "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens) VALUES (?1, ?2, 'file', ?3, ?4, 0.8, ?5)",
                rusqlite::params![
                    buffer_id,
                    summary,
                    serde_json::to_string(&source_chunk_ids)?,
                    source_hash,
                    estimate_tokens(&summary),
                ],
            )?;

            file_summaries += 1;
            self.tracker.update(file_path, file_summaries);

            // Group by module (directory)
            let module = std::path::Path::new(file_path)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("root")
                .to_string();
            module_groups
                .entry(module)
                .or_default()
                .push(summary);
        }

        // Per-module summarization
        let mut module_summaries = 0u32;
        for (module_path, file_summary_texts) in &module_groups {
            let combined = file_summary_texts.join("\n\n");
            let prompt = format!(
                "Summarize the following module at '{}'. Combine these file summaries into a coherent module-level summary:\n\n{}",
                module_path,
                combined
            );

            let summary = self.call_llm(&prompt).await?;

            let source_hash = compute_hash(&summary);

            conn.execute(
                "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens) VALUES (?1, ?2, 'module', '[]', ?3, 0.7, ?4)",
                rusqlite::params![
                    buffer_id,
                    summary,
                    source_hash,
                    estimate_tokens(&summary),
                ],
            )?;

            module_summaries += 1;
        }

        // Per-project summarization
        let all_summaries: Vec<String> = module_groups.values().flat_map(|v| v.clone()).collect();
        let combined = all_summaries.join("\n\n");
        let prompt = format!(
            "Summarize the entire project. Combine these module summaries into a coherent project-level summary:\n\n{}",
            combined
        );

        let project_summary = self.call_llm(&prompt).await?;
        let source_hash = compute_hash(&project_summary);

        conn.execute(
            "INSERT INTO summaries (buffer_id, content, scope, source_chunk_ids, source_hash, confidence, tokens) VALUES (?1, ?2, 'project', '[]', ?3, 0.6, ?4)",
            rusqlite::params![
                buffer_id,
                project_summary,
                source_hash,
                estimate_tokens(&project_summary),
            ],
        )?;

        self.tracker.finish();

        Ok(SummaryResult {
            file_summaries,
            module_summaries,
            project_summaries: 1,
            total_summarized: chunks.len() as u32,
        })
    }

    /// Call the LLM to generate a summary.
    async fn call_llm(&self, prompt: &str) -> Result<String> {
        let messages = vec![
            Message {
                role: Role::System,
                content: "You are a code summarization assistant. Generate concise, accurate summaries of code. Focus on what the code does, its purpose, and key components.".to_string(),
            },
            Message {
                role: Role::User,
                content: prompt.to_string(),
            },
        ];

        let request = CompletionRequest {
            model: "gpt-4o-mini".to_string(),
            messages,
            temperature: Some(0.3),
            max_tokens: Some(1024),
            stop: None,
        };

        let response = self.llm.complete(request).await?;
        Ok(response.content)
    }
}

/// Result of a summarization operation.
#[derive(Debug, Clone)]
pub struct SummaryResult {
    pub file_summaries: u32,
    pub module_summaries: u32,
    pub project_summaries: u32,
    pub total_summarized: u32,
}

/// Compute a simple hash for content.
fn compute_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

/// Estimate token count (rough approximation).
fn estimate_tokens(text: &str) -> u32 {
    // Rough estimate: 1 token per 4 characters
    (text.len() as u32) / 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_summarizer_creation() {
        let tmp = TempDir::new().unwrap();
        let storage = Storage::open(tmp.path()).unwrap();
        let summarizer = Summarizer::new(storage);
        assert!(!summarizer.tracker().is_running());
    }

    #[test]
    fn test_compute_hash() {
        let hash = compute_hash("test content");
        assert_eq!(hash.len(), 64); // SHA-256 hex length
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello"), 1);
        assert_eq!(estimate_tokens("hello world"), 2);
    }
}
