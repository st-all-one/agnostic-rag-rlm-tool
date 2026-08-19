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

#[cfg(test)]
mod tests {
    use super::*;

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
            "".to_string(),
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
}
