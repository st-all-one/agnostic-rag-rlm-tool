#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use arlm_core::*;

#[test]
fn test_rlm_backend_display() {
    assert_eq!(RlmBackend::OpenAi.to_string(), "openai");
    assert_eq!(RlmBackend::Anthropic.to_string(), "anthropic");
}

#[test]
fn test_node_status_display() {
    assert_eq!(NodeStatus::Running.to_string(), "running");
    assert_eq!(NodeStatus::Completed.to_string(), "completed");
}

#[test]
fn test_action_display() {
    assert_eq!(Action::Solve.to_string(), "solve");
    assert_eq!(Action::Decompose.to_string(), "decompose");
}

#[test]
fn test_planner_decision_default() {
    let d = PlannerDecision::default();
    assert_eq!(d.action, Action::Solve);
    assert!(d.subtasks.is_none());
}

#[test]
fn test_rlm_node_running() {
    let node = RlmNode::running("n1", 0, "test task");
    assert_eq!(node.id, "n1");
    assert_eq!(node.depth, 0);
    assert_eq!(node.status, NodeStatus::Running);
    assert!(node.result.is_none());
}

#[test]
fn test_rlm_node_completed() {
    let node = RlmNode::completed("n2", 1, "task", "result".to_string());
    assert_eq!(node.status, NodeStatus::Completed);
    assert_eq!(node.result.as_deref(), Some("result"));
}

#[test]
fn test_rlm_node_failed() {
    let node = RlmNode::failed("n3", 2, "task", "oops".to_string());
    assert_eq!(node.status, NodeStatus::Failed);
    assert_eq!(node.error.as_deref(), Some("oops"));
}

#[test]
fn test_rlm_node_skipped() {
    let node = RlmNode::skipped("n4", 0, "task");
    assert_eq!(node.status, NodeStatus::Skipped);
}

#[test]
fn test_rlm_node_cancelled() {
    let node = RlmNode::cancelled("n5", 0, "task");
    assert_eq!(node.status, NodeStatus::Cancelled);
}

#[test]
fn test_rlm_node_cached() {
    let node = RlmNode::cached("n6", 0, "task", "cached result".to_string());
    assert_eq!(node.status, NodeStatus::Cached);
    assert!(node.cached);
}

#[test]
fn test_rlm_node_with_children() {
    let child = RlmNode::completed("c1", 1, "child", "ok".to_string());
    let parent = RlmNode::running("p1", 0, "parent").with_children(vec![child]);
    assert_eq!(parent.children.len(), 1);
    assert_eq!(parent.children[0].id, "c1");
}

#[test]
fn test_rlm_node_with_decision() {
    let decision = PlannerDecision {
        action: Action::Decompose,
        reason: "complex task".to_string(),
        subtasks: Some(vec!["a".to_string(), "b".to_string()]),
    };
    let node = RlmNode::running("n1", 0, "task").with_decision(decision);
    let d = node.decision.as_ref().expect("decision should exist");
    assert_eq!(d.action, Action::Decompose);
}

#[test]
fn test_rlm_node_finish() {
    let mut node = RlmNode::running("n1", 0, "task");
    node.finish(Some("done".to_string()), None);
    assert_eq!(node.status, NodeStatus::Completed);
    assert!(node.finished_at_ms.is_some());
}

#[test]
fn test_rlm_node_finish_with_error() {
    let mut node = RlmNode::running("n1", 0, "task");
    node.finish(None, Some("error".to_string()));
    assert_eq!(node.status, NodeStatus::Failed);
}

#[test]
fn test_rlm_node_total_usage_empty() {
    let node = RlmNode::running("n1", 0, "task");
    let usage = node.total_usage();
    assert_eq!(usage.cost_usd, 0.0);
    assert_eq!(usage.tokens, 0);
}

#[test]
fn test_rlm_node_total_usage_with_children() {
    let mut child = RlmNode::completed("c1", 1, "child", "ok".to_string());
    child.usage = NodeUsage {
        cost_usd: 0.1,
        tokens: 100,
        errors: 0,
    };
    let mut parent = RlmNode::running("p1", 0, "parent");
    parent.usage = NodeUsage {
        cost_usd: 0.05,
        tokens: 50,
        errors: 0,
    };
    parent.children = vec![child];
    let total = parent.total_usage();
    assert!((total.cost_usd - 0.15).abs() < f64::EPSILON);
    assert_eq!(total.tokens, 150);
}

#[test]
fn test_abort_signal() {
    let signal = AbortSignal::new();
    assert!(!signal.is_cancelled());
    signal.cancel();
    assert!(signal.is_cancelled());
}

#[test]
fn test_now_ms() {
    let ms = now_ms();
    assert!(ms > 0);
}

#[test]
fn test_start_run_input_default() {
    let input = StartRunInput::default();
    assert_eq!(input.max_depth, 3);
    assert_eq!(input.max_nodes, 50);
    assert_eq!(input.concurrency, 4);
}

#[test]
fn test_compaction_policy_default() {
    let p = CompactionPolicy::default();
    assert!(p.enabled);
    assert_eq!(p.max_child_tokens, 8_000);
}

#[test]
fn test_rlm_run_result_serialization() {
    let result = RlmRunResult {
        run_id: "test".to_string(),
        backend: "openai".to_string(),
        final_output: "output".to_string(),
        root: RlmNode::completed("n1", 0, "task", "result".to_string()),
        stats: RunStats::default(),
    };
    let json = serde_json::to_string(&result).expect("serialize");
    assert!(json.contains("test"));
}

#[test]
fn test_custom_tool_function() {
    let tool = CustomTool::function("search_code", "Search the codebase");
    assert_eq!(tool.name, "search_code");
    assert!(tool.callable);
    assert!(tool.parameters.is_none());
}

#[test]
fn test_custom_tool_with_parameters() {
    let tool = CustomTool::function("read_file", "Read a file").with_parameters("path: str");
    assert_eq!(tool.parameters.as_deref(), Some("path: str"));
}

#[test]
fn test_custom_tool_data() {
    let tool = CustomTool::data("api_url", "The API base URL");
    assert!(!tool.callable);
}

#[test]
fn test_format_tools_for_prompt_empty() {
    let result = format_tools_for_prompt(&[]);
    assert!(result.is_empty());
}

#[test]
fn test_format_tools_for_prompt_with_tools() {
    let tools = vec![
        CustomTool::function("search", "Search code"),
        CustomTool::data("config", "App config"),
    ];
    let result = format_tools_for_prompt(&tools);
    assert!(result.contains("Available tools:"));
    assert!(result.contains("- search → Search code [function]"));
    assert!(result.contains("- config → App config [data]"));
}

#[test]
fn test_format_tools_for_prompt_with_parameters() {
    let tools = vec![CustomTool::function("read", "Read file").with_parameters("path: str")];
    let result = format_tools_for_prompt(&tools);
    assert!(result.contains("- read(path: str) → Read file [function]"));
}

#[test]
fn test_start_run_input_default_has_empty_custom_tools() {
    let input = StartRunInput::default();
    assert!(input.custom_tools.is_empty());
}
