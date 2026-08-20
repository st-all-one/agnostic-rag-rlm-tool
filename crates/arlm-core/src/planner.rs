use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::logging::ScopedTimer;
use crate::types::{Action, PlannerDecision, StartRunInput, format_tools_for_prompt};

const PLANNER_SYSTEM: &str = "You are a recursion controller for an RLM system. Analyze the task and decide whether to solve it directly or decompose it into subtasks.";

const ORCHESTRATOR_ADDENDUM: &str = "\
You are an orchestrator, NOT a solver. Your role:
1. Analyze the task and plan a decomposition strategy
2. Break complex tasks into focused subtasks (each with clear, self-contained scope)
3. Respect budget constraints — prefer fewer, well-scoped subtasks over many small ones
4. Reserve your own tokens for high-level decisions, not detailed implementation
5. Delegate detailed work to child nodes via decompose action
6. When choosing solve, ensure you can complete the task in a single response";

/// Build the system prompt, optionally including the orchestrator addendum and custom tools.
#[must_use]
pub fn build_system_prompt(
    include_orchestrator: bool,
    custom_tools: &[crate::types::CustomTool],
) -> String {
    let mut parts = Vec::new();
    parts.push(PLANNER_SYSTEM.to_string());

    if include_orchestrator {
        parts.push(ORCHESTRATOR_ADDENDUM.to_string());
    }

    let tools_block = format_tools_for_prompt(custom_tools);
    if !tools_block.is_empty() {
        parts.push(tools_block);
    }

    parts.join("\n\n")
}

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

    let include_orchestrator = input.mode == crate::types::RlmMode::Auto;
    let system_content = build_system_prompt(include_orchestrator, &input.custom_tools);

    let messages = vec![
        Message {
            role: Role::System,
            content: system_content,
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
pub fn extract_json(text: &str) -> &str {
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
