use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

/// Root-level compactor that summarizes accumulated output when context gets too large.
///
/// By default it keeps recent truncated outputs (non-LLM fallback). When a summarizer
/// [`RootCompactor::summarize_with_llm`] is invoked it asks the LLM to produce a concise
/// summary of all accumulated outputs, reducing many outputs to a single compact block.
pub struct RootCompactor {
    /// Accumulated output summaries.
    summaries: Vec<String>,
    /// Maximum summaries to keep.
    max_summaries: usize,
}

impl RootCompactor {
    /// Create a new root compactor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            summaries: Vec::new(),
            max_summaries: 10,
        }
    }

    /// Add an output to the compactor.
    pub fn add_output(&mut self, output: &str) {
        // Truncate long outputs
        let truncated = if output.len() > 1000 {
            format!("{}...", &output[..1000])
        } else {
            output.to_string()
        };
        self.summaries.push(truncated);

        // Keep only the most recent summaries
        if self.summaries.len() > self.max_summaries {
            let drain = self.summaries.len() - self.max_summaries;
            self.summaries.drain(..drain);
        }
    }

    /// Get a summary of all accumulated outputs (non-LLM fallback).
    #[must_use]
    pub fn get_summary(&self) -> String {
        if self.summaries.is_empty() {
            return "No outputs accumulated.".to_string();
        }

        format!(
            "Accumulated outputs ({}):\n{}",
            self.summaries.len(),
            self.summaries.join("\n---\n")
        )
    }

    /// Produce a concise LLM-generated summary of all accumulated outputs.
    ///
    /// Keeps the non-LLM fallback if the summarization call fails. Returns the number
    /// of source outputs that were summarized.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM summarization call fails.
    pub async fn summarize_with_llm(
        &self,
        llm: &Arc<dyn LlmBackend + Send + Sync>,
        model: &str,
        retry_config: &arlm_llm::RetryConfig,
    ) -> Result<String> {
        if self.summaries.is_empty() {
            return Ok("No outputs accumulated.".to_string());
        }

        let prompt = format!(
            "Summarize the following accumulated RLM root outputs into a single concise block \
             preserving key results, decisions, and facts:\n\n{}",
            self.summaries.join("\n---\n")
        );

        let request = CompletionRequest {
            model: model.to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: "You are a concise summarizer. Preserve key results and decisions."
                        .to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: Some(0.2),
            max_tokens: Some(1024),
            stop: None,
        };

        let response = retry_with_backoff(retry_config, || {
            let req = request.clone();
            let llm = llm.clone();
            async move { llm.complete(req).await }
        })
        .await?;

        info!(model = model, "root compaction summarized via LLM");
        Ok(format!("[Root summary]\n{}", response.content))
    }

    /// Clear the summaries.
    pub fn clear(&mut self) {
        self.summaries.clear();
    }

    /// Get the number of accumulated outputs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.summaries.len()
    }

    /// Check if the compactor is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.summaries.is_empty()
    }
}

impl Default for RootCompactor {
    fn default() -> Self {
        Self::new()
    }
}
