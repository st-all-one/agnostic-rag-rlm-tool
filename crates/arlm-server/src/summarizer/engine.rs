//! The [`Summarizer`] engine: loads chunks, calls the LLM and persists the
//! resulting hierarchical summaries while streaming progress to the hub.

use std::collections::HashMap;

use anyhow::{Context, Result};
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role};
use arlm_storage::Storage;

use crate::events::EventHub;
use crate::store;
use crate::timing::Timer;

use super::progress::ProgressTracker;
use super::strategy::{RawChunk, build_summary_prompt};
use super::{SummarizeJob, SummaryResult};

/// Summarization engine bound to a specific storage, LLM backend and event hub.
pub struct Summarizer {
    storage: Storage,
    llm: std::sync::Arc<dyn LlmBackend + Send + Sync>,
    events: EventHub,
    tracker: ProgressTracker,
}

impl Summarizer {
    /// Create a new summarizer.
    #[must_use]
    pub fn new(
        storage: Storage,
        llm: std::sync::Arc<dyn LlmBackend + Send + Sync>,
        events: EventHub,
    ) -> Self {
        Self {
            storage,
            llm,
            events,
            tracker: ProgressTracker::new(),
        }
    }

    /// Get the progress tracker.
    #[must_use]
    pub fn tracker(&self) -> &ProgressTracker {
        &self.tracker
    }

    /// Run a summarization job, persisting file, module and project summaries.
    ///
    /// # Errors
    ///
    /// Returns an error if chunk loading, LLM calls or persistence fail.
    pub async fn summarize(&self, job: SummarizeJob) -> Result<SummaryResult> {
        let _timer = Timer::new("summarize_project");
        self.events.register_summarize(&job.run_id);

        let chunks = self.load_chunks(job.buffer_id)?;
        if chunks.is_empty() {
            self.events.unregister_summarize(&job.run_id);
            return Ok(SummaryResult::default());
        }

        let file_count = chunks
            .iter()
            .map(|c| c.file_path.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len();
        self.tracker.start(u32::try_from(file_count).unwrap_or(0));

        // Group chunks by file.
        let mut file_groups: HashMap<String, Vec<&RawChunk>> = HashMap::new();
        for chunk in &chunks {
            file_groups
                .entry(chunk.file_path.clone())
                .or_default()
                .push(chunk);
        }

        let mut module_groups: HashMap<String, Vec<String>> = HashMap::new();
        let mut file_summaries = 0u32;

        // File scope (only when the requested scope reaches at least files).
        if job.max_scope >= 0 {
            for (file_path, file_chunks) in &file_groups {
                let owned: Vec<RawChunk> = file_chunks.iter().map(|c| (*c).clone()).collect();
                let summary = self.summarize_group(&owned, "file").await?;
                self.persist(
                    job.buffer_id,
                    &summary,
                    "file",
                    owned.iter().map(|c| c.id).collect(),
                    owned
                        .iter()
                        .map(|c| c.content.as_str())
                        .collect::<Vec<_>>()
                        .join(""),
                    0.8,
                )?;

                file_summaries += 1;
                self.tracker.update(file_path, file_summaries);

                let module = std::path::Path::new(file_path)
                    .parent()
                    .and_then(|p| p.to_str())
                    .unwrap_or("root")
                    .to_string();
                module_groups.entry(module).or_default().push(summary);
                self.emit_progress(
                    &job.run_id,
                    file_path,
                    file_summaries,
                    u32::try_from(file_groups.len()).unwrap_or(0),
                    format!("summarizing {file_path}"),
                );
            }
        }

        // Module scope.
        let mut module_summaries = 0u32;
        if job.max_scope >= 1 {
            for (module_path, file_summary_texts) in &module_groups {
                let combined = file_summary_texts.join("\n\n");
                let summary = self
                    .call_llm(&format!(
                        "Summarize the module at '{module_path}'. Combine these file summaries into a coherent module-level summary:\n\n{combined}"
                    ))
                    .await?;
                self.persist(
                    job.buffer_id,
                    &summary,
                    "module",
                    Vec::new(),
                    module_path.clone(),
                    0.7,
                )?;
                module_summaries += 1;
                self.emit_progress(
                    &job.run_id,
                    module_path,
                    module_summaries,
                    module_groups.len() as u32,
                    format!("summarizing module {module_path}"),
                );
            }
        }

        // Project scope.
        let mut project_summaries = 0u32;
        if job.max_scope >= 2 {
            let all = module_groups
                .values()
                .flat_map(|v| v.clone())
                .collect::<Vec<_>>()
                .join("\n\n");
            let summary = self
                .call_llm(&format!(
                    "Summarize the entire project. Combine these module summaries into a coherent project-level summary:\n\n{all}"
                ))
                .await?;
            self.persist(
                job.buffer_id,
                &summary,
                "project",
                Vec::new(),
                "project".to_string(),
                0.6,
            )?;
            project_summaries = 1;
            self.emit_progress(
                &job.run_id,
                "project",
                1,
                1,
                "summarizing project".to_string(),
            );
        }

        self.tracker.finish();
        self.events.unregister_summarize(&job.run_id);

        Ok(SummaryResult {
            file_summaries,
            module_summaries,
            project_summaries,
            total_summarized: u32::try_from(chunks.len()).unwrap_or(u32::MAX),
        })
    }

    /// Summarize a group of chunks with a single LLM call.
    async fn summarize_group(&self, file_chunks: &[RawChunk], scope: &str) -> Result<String> {
        let prompt = build_summary_prompt(file_chunks, scope)?;
        self.call_llm(&prompt).await
    }

    /// Persist a summary row.
    fn persist(
        &self,
        buffer_id: i64,
        summary: &str,
        scope: &str,
        source_chunk_ids: Vec<i64>,
        source_text: String,
        confidence: f64,
    ) -> Result<()> {
        let source_hash = super::compute_hash(&source_text);
        let source_json = serde_json::to_string(&source_chunk_ids)?;
        let tokens = super::estimate_tokens(summary);
        store::insert_summary(
            &self.storage,
            buffer_id,
            summary,
            scope,
            &source_json,
            &source_hash,
            confidence,
            tokens,
        )
    }

    /// Load all chunks of a buffer with their text content.
    fn load_chunks(&self, buffer_id: i64) -> Result<Vec<RawChunk>> {
        let conn = self.storage.connection()?;
        conn.execute(|conn| {
            let mut stmt = conn.prepare(
                "SELECT c.id, COALESCE(cc.content, ''), c.file_path \
                 FROM chunks c LEFT JOIN chunk_texts cc ON cc.chunk_id = c.id \
                 WHERE c.buffer_id = ?1 ORDER BY c.file_path, c.line_start",
            )?;
            let rows = stmt.query_map(rusqlite::params![buffer_id], |row| {
                Ok(RawChunk {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    file_path: row.get(2)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .context("failed to load chunks for buffer")
    }

    /// Call the configured LLM backend with a summarization prompt.
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
            model: self.model_name(),
            messages,
            temperature: Some(0.3),
            max_tokens: Some(1024),
            stop: None,
        };

        let response = self.llm.complete(request).await?;
        Ok(response.content)
    }

    fn model_name(&self) -> String {
        // The server config model is carried on the client; the backend picks
        // its default when the request model is empty. Prefer an explicit model
        // injected through the LLM backend factory when available.
        String::new()
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_progress(
        &self,
        run_id: &str,
        current_file: &str,
        completed: u32,
        total: u32,
        message: String,
    ) {
        self.events
            .publish_summarize(arlm_proto::proto::SummarizeProgress {
                run_id: run_id.to_string(),
                current_scope: 0,
                current_file: current_file.to_string(),
                completed: i32::try_from(completed).unwrap_or(i32::MAX),
                total: i32::try_from(total).unwrap_or(i32::MAX),
                elapsed_ms: 0.0,
                message,
            });
    }
}
