use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{debug, instrument};

use arlm_core::{RlmEvent, RlmRunResult};

use crate::commands::run::config::RunConfig;
use crate::output::LiveTree;

/// Run the RLM engine in live mode, streaming events into a live-updating tree
/// view rendered to the terminal every ~100ms.
#[allow(clippy::missing_errors_doc, clippy::cast_possible_truncation)]
#[instrument(skip(config, input, llm_backend), fields(run_id = %input.run_id))]
pub async fn run_live(
    config: &RunConfig<'_>,
    input: arlm_core::StartRunInput,
    llm_backend: Arc<dyn arlm_llm::LlmBackend>,
) -> Result<RlmRunResult> {
    let frame_start = std::time::Instant::now();
    debug!(
        task = %config.task,
        "starting live RLM rendering (real-time recursion tree)"
    );
    let event_bus = arlm_core::EventBus::new();
    let mut rx = event_bus.subscribe();
    let mut tree = LiveTree::new();

    let render_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let mut batch = Vec::with_capacity(8);
                    while let Ok(event) = rx.try_recv() {
                        batch.push(event);
                    }
                    if !batch.is_empty() {
                        let render_start = std::time::Instant::now();
                        for event in &batch {
                            tree.apply(event);
                        }
                        print!("\x1B[2J\x1B[1;1H");
                        let rendered = tree.render();
                        println!("{rendered}");
                        debug!(
                            events = batch.len(),
                            elapsed_ms = render_start.elapsed().as_millis() as u64,
                            "rendered live frame"
                        );
                    }
                }
            }
        }
    });

    let result = arlm_core::run_rlm_engine_with_events(input, llm_backend, event_bus, None)
        .await
        .context("RLM engine failed")?;

    render_handle.abort();

    print!("\x1B[2J\x1B[1;1H");
    let mut tree = LiveTree::new();
    tree.apply(&RlmEvent::RunStart {
        run_id: Arc::from(result.run_id.as_str()),
        task: config.task.to_string(),
        backend: config.backend.unwrap_or("ollama").to_string(),
        mode: "auto".to_string(),
        max_depth: config.depth,
        max_nodes: config.max_nodes,
        max_budget: config.max_budget,
        started_at_ms: arlm_core::now_ms(),
    });
    println!("{}", tree.render());
    println!("\n{}", result.final_output);
    debug!(
        elapsed_ms = frame_start.elapsed().as_millis() as u64,
        "live run complete"
    );
    Ok(result)
}
