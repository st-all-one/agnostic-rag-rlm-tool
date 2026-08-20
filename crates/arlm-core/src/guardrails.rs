use std::collections::HashSet;

/// Normalize a task string for comparison (lowercase, single spaces).
#[must_use]
pub fn normalize_task(task: &str) -> String {
    task.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Detect if a task creates a cycle in the lineage.
#[must_use]
pub fn detect_cycle(task: &str, lineage: &[String]) -> bool {
    let normalized = normalize_task(task);
    lineage.iter().any(|l| l == &normalized)
}

/// Sanitize subtasks: trim, deduplicate, remove empty and parent-equivalent.
#[must_use]
pub fn sanitize_subtasks(subtasks: &[String], parent_task: &str) -> Vec<String> {
    let parent_normalized = normalize_task(parent_task);
    let mut seen = HashSet::new();
    subtasks
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .filter(|s| normalize_task(s) != parent_normalized)
        .filter(|s| seen.insert(normalize_task(s)))
        .collect()
}
