use std::fmt::Write;
use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::logging::ScopedTimer;
use crate::token_counter::get_context_limit;
use crate::types::{Action, CompactionPolicy, NodeStatus, RlmNode, StartRunInput};

const SYNTHESIZER_SYSTEM: &str = "You are a synthesizer in an RLM system. Merge child outputs into one coherent, complete answer.";

/// Share of the model context limit beyond which child outputs are compacted (gap #4).
const CHILD_COMPACTION_CONTEXT_FRACTION: f64 = 0.85;

/// Synthesize child node results into a single answer.
///
/// Applies token-based compaction (#4) of child outputs respecting [`CompactionPolicy`]
/// (#5) before formatting the block and calling the synthesizer LLM.
///
/// # Errors
///
/// Returns an error if the LLM call fails.
pub async fn synthesize(
    parent_task: &str,
    children: &[RlmNode],
    input: &StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    model_override: Option<&str>,
) -> Result<String> {
    let _timer = ScopedTimer::new("synthesize");

    let compacted_children =
        compact_children_if_needed(children, &input.compaction, &llm, budget, model_override)
            .await?;

    let children_block = build_children_block(&compacted_children);

    let prompt = format!(
        "You are the synthesizer node. Merge the outputs of child nodes into one coherent answer.\n\n\
         Parent task: {parent_task}\n\n\
         Children outputs:\n{children_block}\n\
         Synthesize a unified, complete answer. Handle failed/cancelled children gracefully.",
    );

    let model = model_override
        .or(input.model.as_deref())
        .unwrap_or("gpt-4o")
        .to_string();

    let messages = vec![
        Message {
            role: Role::System,
            content: SYNTHESIZER_SYSTEM.to_string(),
        },
        Message {
            role: Role::User,
            content: prompt,
        },
    ];

    let sampling = crate::sampling::SamplingArgs::for_node_type(Action::Solve);
    let request = sampling.apply_to_request(CompletionRequest {
        model: model.clone(),
        messages,
        temperature: None,
        max_tokens: Some(4096),
        stop: None,
    });

    let response = retry_with_backoff(&input.retry_policy.inner, || {
        let req = request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(&model, &response.usage);

    info!(
        parent_task = parent_task,
        children = children.len(),
        "synthesized"
    );

    Ok(response.content)
}

/// Build a formatted block of children outputs for the synthesizer prompt.
#[must_use]
pub fn build_children_block(children: &[RlmNode]) -> String {
    let mut block = String::new();
    for (i, child) in children.iter().enumerate() {
        let _ = writeln!(
            block,
            "--- Child {} (depth {}, status: {}) ---",
            i + 1,
            child.depth,
            child.status
        );
        match child.status {
            NodeStatus::Completed | NodeStatus::Cached => {
                if let Some(result) = &child.result {
                    block.push_str(result);
                } else {
                    block.push_str("[no result]");
                }
            }
            NodeStatus::Failed => {
                let _ = write!(
                    block,
                    "[FAILED: {}]",
                    child.error.as_deref().unwrap_or("unknown")
                );
            }
            NodeStatus::Cancelled => {
                block.push_str("[CANCELLED]");
                if let Some(partial) = &child.partial_answer {
                    let _ = write!(block, "\nPartial: {partial}");
                }
            }
            NodeStatus::Skipped => {
                block.push_str("[SKIPPED: budget exhausted]");
            }
            NodeStatus::Running => {
                block.push_str("[RUNNING]");
            }
        }
        block.push('\n');
    }
    block
}

/// Token cost of a single child's rendered result (used by compaction decisions).
fn child_token_cost(child: &RlmNode) -> u32 {
    let text = child
        .result
        .as_deref()
        .or(child.partial_answer.as_deref())
        .unwrap_or("");
    crate::token_counter::TokenCounter::estimate(text).max(1)
}

/// Compact the oldest child outputs via an LLM summary when they exceed the budget.
///
/// Returns the (possibly) reduced list of children. When `policy.enabled` is false or the
/// accumulated child tokens fit within the threshold, children are returned unchanged.
async fn compact_children_if_needed(
    children: &[RlmNode],
    policy: &CompactionPolicy,
    llm: &Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    model_override: Option<&str>,
) -> Result<Vec<RlmNode>> {
    if !policy.enabled || children.len() <= 1 {
        return Ok(children.to_vec());
    }

    let model = model_override.unwrap_or("gpt-4o").to_string();
    let model_limit = get_context_limit(&model);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let context_threshold = (f64::from(model_limit) * CHILD_COMPACTION_CONTEXT_FRACTION) as u32;
    let threshold = context_threshold.min(policy.max_child_tokens).max(1);

    let costs: Vec<u32> = children.iter().map(child_token_cost).collect();
    let total: u32 = costs.iter().sum();

    if total <= threshold {
        return Ok(children.to_vec());
    }

    // Greedily keep the newest children that fit; compact the oldest that don't.
    let mut kept: Vec<RlmNode> = Vec::new();
    let mut to_compact: Vec<&RlmNode> = Vec::new();
    let mut running: u32 = 0;
    for (i, child) in children.iter().enumerate() {
        if running + costs[i] <= threshold {
            kept.push(child.clone());
            running += costs[i];
        } else {
            to_compact.push(child);
        }
    }

    // Always retain at least the newest child.
    if kept.is_empty() && !to_compact.is_empty() {
        let newest = to_compact.remove(to_compact.len() - 1);
        kept.push(newest.clone());
    }

    if to_compact.is_empty() {
        return Ok(kept);
    }

    let summary = summarize_children(&to_compact, &model, llm, budget).await?;
    let summarized = RlmNode::cached(
        &format!("compacted-{}", children.len()),
        1,
        "compacted child outputs",
        summary,
    );
    kept.push(summarized);
    info!(compacted = to_compact.len(), "compacted oldest children outputs");
    Ok(kept)
}

/// Summarize a set of child nodes into a single compact text block via the LLM.
async fn summarize_children(
    children: &[&RlmNode],
    model: &str,
    llm: &Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
) -> Result<String> {
    let mut body = String::new();
    for (i, child) in children.iter().enumerate() {
        let text = child
            .result
            .as_deref()
            .or(child.partial_answer.as_deref())
            .unwrap_or("[no result]");
        let _ = writeln!(body, "--- Child {} (status: {}) ---\n{text}", i + 1, child.status);
    }

    let prompt = format!(
        "The following child node outputs must be compacted because the parent context is full. \
         Produce a concise summary preserving key facts, results, and decisions:\n\n{body}"
    );

    let request = CompletionRequest {
        model: model.to_string(),
        messages: vec![
            Message {
                role: Role::System,
                content: "You are a concise summarizer for an RLM synthesizer.".to_string(),
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

    let response = retry_with_backoff(&arlm_llm::RetryConfig::default(), || {
        let req = request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(model, &response.usage);
    Ok(response.content)
}

