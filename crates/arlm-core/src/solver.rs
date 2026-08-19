use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::cache::ResultCache;
use crate::logging::ScopedTimer;
use crate::types::{Action, StartRunInput};

const SOLVER_SYSTEM: &str = "You are a worker node in an RLM system. Solve the task directly and return a concrete, actionable answer.";

/// Solve a task directly by calling the LLM.
///
/// # Errors
///
/// Returns an error if the LLM call fails.
pub async fn solve_task(
    task: &str,
    input: &StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    cache: &ResultCache,
    forced_reason: Option<&str>,
) -> Result<String> {
    let _timer = ScopedTimer::new("solve_task");

    if input.enable_cache {
        if let Some(cached) = cache.get(task, &input.project) {
            info!(task = task, "cache hit");
            return Ok(cached);
        }
    }

    let prompt = if let Some(reason) = forced_reason {
        format!(
            "Solve this task directly. You were forced to solve because: {reason}\n\nTask: {task}\n\nProvide a concrete, actionable answer.",
        )
    } else {
        format!("Solve this task directly and return a concrete answer.\n\nTask: {task}")
    };

    let model = input.model.clone().unwrap_or_else(|| "gpt-4o".to_string());

    let messages = vec![
        Message {
            role: Role::System,
            content: SOLVER_SYSTEM.to_string(),
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
        max_tokens: Some(2048),
        stop: None,
    });

    let response = retry_with_backoff(&input.retry_policy.inner, || {
        let req = request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(&model, &response.usage);

    if input.enable_cache {
        cache.put(task, &input.project, &response.content);
    }

    info!(
        task = task,
        tokens = response.usage.total_tokens,
        "task solved"
    );

    Ok(response.content)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    #[test]
    fn test_solver_prompt_with_forced_reason() {
        let task = "implement feature X";
        let reason = "max depth reached";
        let prompt = format!(
            "Solve this task directly. You were forced to solve because: {reason}

Task: {task}

Provide a concrete, actionable answer.",
        );
        assert!(prompt.contains("forced to solve"));
        assert!(prompt.contains(task));
    }

    #[test]
    fn test_solver_prompt_without_forced_reason() {
        let task = "fix bug Y";
        let prompt = format!(
            "Solve this task directly and return a concrete answer.

Task: {task}",
        );
        assert!(!prompt.contains("forced"));
        assert!(prompt.contains(task));
    }
}
