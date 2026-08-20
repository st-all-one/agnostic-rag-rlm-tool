#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use arlm_core::guardrails::{detect_cycle, normalize_task, sanitize_subtasks};

#[test]
fn test_normalize_task() {
    assert_eq!(normalize_task("  Hello   World  "), "hello world");
    assert_eq!(normalize_task("TASK"), "task");
    assert_eq!(normalize_task("  a  b  c  "), "a b c");
}

#[test]
fn test_normalize_task_empty() {
    assert_eq!(normalize_task(""), "");
    assert_eq!(normalize_task("   "), "");
}

#[test]
fn test_detect_cycle_no_cycle() {
    let lineage = vec!["task a".to_string(), "task b".to_string()];
    assert!(!detect_cycle("task c", &lineage));
}

#[test]
fn test_detect_cycle_with_cycle() {
    let lineage = vec!["task a".to_string(), "task b".to_string()];
    assert!(detect_cycle("Task A", &lineage));
}

#[test]
fn test_detect_cycle_empty_lineage() {
    assert!(!detect_cycle("anything", &[]));
}

#[test]
fn test_sanitize_subtasks_basic() {
    let subtasks = vec![
        "  Subtask 1  ".to_string(),
        "Subtask 2".to_string(),
        String::new(),
        "   ".to_string(),
    ];
    let result = sanitize_subtasks(&subtasks, "parent task");
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], "Subtask 1");
    assert_eq!(result[1], "Subtask 2");
}

#[test]
fn test_sanitize_subtasks_removes_parent() {
    let subtasks = vec!["Parent Task".to_string(), "Child Task".to_string()];
    let result = sanitize_subtasks(&subtasks, "Parent Task");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], "Child Task");
}

#[test]
fn test_sanitize_subtasks_deduplicates() {
    let subtasks = vec![
        "Same Task".to_string(),
        "same task".to_string(),
        "SAME TASK".to_string(),
    ];
    let result = sanitize_subtasks(&subtasks, "other");
    assert_eq!(result.len(), 1);
}
