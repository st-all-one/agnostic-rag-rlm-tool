//! Persistent summarization worker.
//!
//! Runs as a background task that consumes [`SummarizeJob`]s from an unbounded
//! channel. Keeps the summarizer warm across calls instead of spawning a new
//! one per request (TODO gap #18).

use std::sync::Arc;

use anyhow::Result;
use arlm_llm::LlmBackend;
use arlm_storage::Storage;
use tokio::sync::mpsc;

use crate::events::EventHub;
use crate::timing::Timer;

use super::{SummarizeJob, Summarizer};

/// Handle that enqueues jobs for the background worker.
pub type SummarizeSender = mpsc::UnboundedSender<SummarizeJob>;

/// Spawn the background summarization worker and return its queue handle.
#[must_use]
pub fn spawn_worker(
    storage: Storage,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    events: EventHub,
) -> SummarizeSender {
    let (tx, mut rx) = mpsc::unbounded_channel::<SummarizeJob>();

    tokio::spawn(async move {
        tracing::info!("summarization worker started");
        let summarizer = Summarizer::new(storage, llm, events);

        while let Some(job) = rx.recv().await {
            let _timer = Timer::new("summarize_job");
            match summarizer.summarize(job.clone()).await {
                Ok(result) => {
                    tracing::info!(
                        run_id = %job.run_id,
                        file_summaries = result.file_summaries,
                        module_summaries = result.module_summaries,
                        project_summaries = result.project_summaries,
                        "summarization completed"
                    );
                }
                Err(e) => {
                    tracing::error!(run_id = %job.run_id, error = %e, "summarization failed");
                }
            }
        }

        tracing::info!("summarization worker stopped");
    });

    tx
}

/// Convenience wrapper for callers that need a typed result, if any.
///
/// Reserved for future use; the worker logs results directly.
pub async fn run_job(
    storage: Storage,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    events: EventHub,
    job: SummarizeJob,
) -> Result<super::SummaryResult> {
    let summarizer = Summarizer::new(storage, llm, events);
    summarizer.summarize(job).await
}
