use std::fmt::Write;
use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::logging::ScopedTimer;
use crate::types::{NodeStatus, RlmNode, StartRunInput};

const SYNTHESIZER_SYSTEM: &str = "You are a synthesizer in an RLM system. Merge child outputs into one coherent, complete answer.";

/// Synthesize child node results into a single answer.
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
) -> Result<String> {
    let _timer = ScopedTimer::new("synthesize");

    let children_block = build_children_block(children);

    let prompt = format!(
        "You are the synthesizer node. Merge the outputs of child nodes into one coherent answer.\n\n\
         Parent task: {parent_task}\n\n\
         Children outputs:\n{children_block}\n\
         Synthesize a unified, complete answer. Handle failed/cancelled children gracefully.",
    );

    let model = input.model.clone().unwrap_or_else(|| "gpt-4o".to_string());

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

    let request = CompletionRequest {
        model: model.clone(),
        messages,
        temperature: Some(0.3),
        max_tokens: Some(4096),
        stop: None,
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_children_block_completed() {
        let child = RlmNode::completed("c1", 1, "child task", "result text".to_string());
        let block = build_children_block(&[child]);
        assert!(block.contains("result text"));
        assert!(block.contains("completed"));
    }

    #[test]
    fn test_build_children_block_failed() {
        let child = RlmNode::failed("c1", 1, "child task", "error msg".to_string());
        let block = build_children_block(&[child]);
        assert!(block.contains("FAILED"));
        assert!(block.contains("error msg"));
    }

    #[test]
    fn test_build_children_block_cancelled() {
        let mut child = RlmNode::cancelled("c1", 1, "child task");
        child.partial_answer = Some("partial result".to_string());
        let block = build_children_block(&[child]);
        assert!(block.contains("CANCELLED"));
        assert!(block.contains("partial result"));
    }

    #[test]
    fn test_build_children_block_skipped() {
        let child = RlmNode::skipped("c1", 1, "child task");
        let block = build_children_block(&[child]);
        assert!(block.contains("SKIPPED"));
    }

    #[test]
    fn test_build_children_block_multiple() {
        let children = vec![
            RlmNode::completed("c1", 1, "t1", "r1".to_string()),
            RlmNode::failed("c2", 1, "t2", "e2".to_string()),
        ];
        let block = build_children_block(&children);
        assert!(block.contains("Child 1"));
        assert!(block.contains("Child 2"));
    }
}
