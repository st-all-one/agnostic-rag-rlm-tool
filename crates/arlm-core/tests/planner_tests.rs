#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::Action;
use arlm_core::planner::{build_system_prompt, extract_json, parse_planner_decision};

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

#[test]
fn test_build_system_prompt_orchestrator() {
    let prompt = build_system_prompt(true, &[]);
    assert!(prompt.contains("orchestrator"));
    assert!(prompt.contains("NOT a solver"));
    assert!(prompt.contains("recursion controller"));
}

#[test]
fn test_build_system_prompt_no_orchestrator() {
    let prompt = build_system_prompt(false, &[]);
    assert!(prompt.contains("recursion controller"));
    assert!(!prompt.contains("orchestrator"));
}

#[test]
fn test_build_system_prompt_with_tools() {
    let tools = vec![arlm_core::CustomTool::function("search", "Search code")];
    let prompt = build_system_prompt(false, &tools);
    assert!(prompt.contains("Available tools:"));
    assert!(prompt.contains("search"));
}

#[test]
fn test_build_system_prompt_orchestrator_with_tools() {
    let tools = vec![arlm_core::CustomTool::function("read", "Read file")];
    let prompt = build_system_prompt(true, &tools);
    assert!(prompt.contains("orchestrator"));
    assert!(prompt.contains("Available tools:"));
    assert!(prompt.contains("read"));
}
