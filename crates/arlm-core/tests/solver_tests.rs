#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

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
