use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::logging::ScopedTimer;
use crate::types::{Action, PlannerDecision, StartRunInput};

const PLANNER_SYSTEM: &str = "You are a recursion controller for an RLM system. Analyze the task and decide whether to solve it directly or decompose it into subtasks.";

/// Plan a node: call LLM to decide solve vs decompose.
///
/// # Errors
///
/// Returns an error if the LLM call fails or the response cannot be parsed.
pub async fn plan_node(
    task: &str,
    depth: u32,
    input: &StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    nodes_visited: u32,
    model_override: Option<&str>,
) -> Result<PlannerDecision> {
    let _timer = ScopedTimer::new("plan_node");
    let summary = budget.summary();

    let prompt = format!(
        r#"Analyze the task and decide whether to solve it directly or decompose it into subtasks.

Task: {task}

Context:
- Depth: {depth}/{max_depth}
- Nodes visited: {visited}/{max_nodes}
- Remaining budget: ${budget:.2} / {tokens} tokens / {errors} errors / {time}s

Return JSON: {{"action": "solve"|"decompose", "reason": "...", "subtasks": ["..."]}}

If the task is atomic or budget is low, choose "solve".
If the task can be meaningfully split, choose "decompose" with 2-5 subtasks.
Decomposition multiplies cost — only decompose when it clearly helps."#,
        task = task,
        depth = depth,
        max_depth = input.max_depth,
        visited = nodes_visited,
        max_nodes = input.max_nodes,
        budget = summary.budget_remaining,
        tokens = summary.tokens_remaining,
        errors = summary.errors_remaining,
        time = summary.time_remaining_ms / 1000,
    );

    let model = model_override
        .or(input.model.as_deref())
        .unwrap_or("gpt-4o")
        .to_string();

    let messages = vec![
        Message {
            role: Role::System,
            content: PLANNER_SYSTEM.to_string(),
        },
        Message {
            role: Role::User,
            content: prompt,
        },
    ];

    let sampling = crate::sampling::SamplingArgs::for_node_type(Action::Decompose);
    let request = sampling.apply_to_request(CompletionRequest {
        model: model.clone(),
        messages,
        temperature: None,
        max_tokens: Some(512),
        stop: None,
    });

    let response = retry_with_backoff(&input.retry_policy.inner, || {
        let req = request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(&model, &response.usage);

    let decision = parse_planner_decision(&response.content);
    info!(
        action = decision.action.to_string(),
        reason = %decision.reason,
        "planner decision"
    );

    Ok(decision)
}

/// Parse the planner's JSON response into a `PlannerDecision`.
///
/// Falls back to "solve" if parsing fails.
#[must_use]
pub fn parse_planner_decision(text: &str) -> PlannerDecision {
    let json_str = extract_json(text);

    match serde_json::from_str::<PlannerDecisionRaw>(json_str) {
        Ok(raw) => PlannerDecision {
            action: match raw.action.as_str() {
                "decompose" => Action::Decompose,
                _ => Action::Solve,
            },
            reason: raw.reason,
            subtasks: raw.subtasks,
        },
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse planner decision, defaulting to solve");
            PlannerDecision {
                action: Action::Solve,
                reason: format!("parse error: {e}"),
                subtasks: None,
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct PlannerDecisionRaw {
    action: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    subtasks: Option<Vec<String>>,
}

/// Extract JSON substring from text (handles markdown code blocks).
#[must_use]
fn extract_json(text: &str) -> &str {
    if let Some(start) = text.find("```json") {
        let json_start = start + 7;
        if let Some(end) = text[json_start..].find("```") {
            return text[json_start..json_start + end].trim();
        }
    }
    if let Some(start) = text.find("```") {
        let json_start = start + 3;
        if let Some(line_end) = text[json_start..].find('\n') {
            let json_start = json_start + line_end + 1;
            if let Some(end) = text[json_start..].find("```") {
                return text[json_start..json_start + end].trim();
            }
        }
    }
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            return &text[start..=end];
        }
    }
    text.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_planner_decision_solve() {
        let json = r#"{"action": "solve", "reason": "atomic task", "subtasks": null}"#;
        let decision = parse_planner_decision(json);
        assert_eq!(decision.action, Action::Solve);
        assert_eq!(decision.reason, "atomic task");
        assert!(decision.subtasks.is_none());
    }

    #[test]
    fn test_parse_planner_decision_decompose() {
        let json = r#"{"action": "decompose", "reason": "complex task", "subtasks": ["a", "b"]}"#;
        let decision = parse_planner_decision(json);
        assert_eq!(decision.action, Action::Decompose);
        assert_eq!(decision.subtasks.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_planner_decision_in_code_block() {
        let text =
            "Here is my analysis:\n```json\n{\"action\": \"solve\", \"reason\": \"simple\"}\n```\n";
        let decision = parse_planner_decision(text);
        assert_eq!(decision.action, Action::Solve);
    }

    #[test]
    fn test_parse_planner_decision_invalid_falls_back_to_solve() {
        let decision = parse_planner_decision("not json at all");
        assert_eq!(decision.action, Action::Solve);
    }

    #[test]
    fn test_extract_json_raw() {
        let text = r#"{"action": "solve", "reason": "test"}"#;
        let json = extract_json(text);
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_extract_json_in_code_block() {
        let text = "```json\n{\"action\": \"solve\"}\n```";
        let json = extract_json(text);
        assert!(json.contains("solve"));
    }
}
