//! Integration tests for the new features.

#[cfg(test)]
mod tests {
    use arags_core::{
        PersistentSolver, StateInspector, RootCompactor,
        types::{StartRunInput, RlmBackend},
    };
    use std::sync::Arc;

    #[test]
    fn test_persistent_solver_creation() {
        let solver = PersistentSolver::new(10_000);
        assert!(solver.history().is_empty());
    }

    #[test]
    fn test_persistent_solver_history() {
        let mut solver = PersistentSolver::new(10_000);
        solver.add_user_message("Hello");
        solver.add_assistant_message("Hi there");
        assert_eq!(solver.history().len(), 2);
    }

    #[test]
    fn test_persistent_solver_clear() {
        let mut solver = PersistentSolver::new(10_000);
        solver.add_user_message("Hello");
        solver.clear();
        assert!(solver.history().is_empty());
    }

    #[test]
    fn test_state_inspector_creation() {
        let inspector = StateInspector::new();
        assert_eq!(inspector.iteration_count(), 0);
        assert!(inspector.completed_tasks().is_empty());
    }

    #[test]
    fn test_state_inspector_record_task() {
        let mut inspector = StateInspector::new();
        inspector.record_task("task1", "result1");
        inspector.record_task("task2", "result2");
        assert_eq!(inspector.iteration_count(), 2);
        assert_eq!(inspector.completed_tasks().len(), 2);
    }

    #[test]
    fn test_state_inspector_variables() {
        let mut inspector = StateInspector::new();
        inspector.set_variable("x", "42");
        inspector.set_variable("y", "hello");
        assert_eq!(inspector.get_variable("x"), Some("42"));
        assert_eq!(inspector.get_variable("y"), Some("hello"));
        assert_eq!(inspector.get_variable("z"), None);
    }

    #[test]
    fn test_state_inspector_show_vars() {
        let mut inspector = StateInspector::new();
        inspector.set_variable("count", "10");
        let output = inspector.show_vars();
        assert!(output.contains("count"));
        assert!(output.contains("10"));
    }

    #[test]
    fn test_state_inspector_summary() {
        let mut inspector = StateInspector::new();
        inspector.record_task("task1", "result1");
        inspector.set_variable("x", "42");
        let summary = inspector.summary();
        assert!(summary.contains("1 tasks"));
        assert!(summary.contains("2 variables"));
    }

    #[test]
    fn test_root_compactor_creation() {
        let compactor = RootCompactor::new();
        assert!(compactor.is_empty());
        assert_eq!(compactor.len(), 0);
    }

    #[test]
    fn test_root_compactor_add_output() {
        let mut compactor = RootCompactor::new();
        compactor.add_output("output1");
        compactor.add_output("output2");
        assert_eq!(compactor.len(), 2);
    }

    #[test]
    fn test_root_compactor_summary() {
        let mut compactor = RootCompactor::new();
        compactor.add_output("first result");
        compactor.add_output("second result");
        let summary = compactor.get_summary();
        assert!(summary.contains("2"));
        assert!(summary.contains("first result"));
        assert!(summary.contains("second result"));
    }

    #[test]
    fn test_root_compactor_clear() {
        let mut compactor = RootCompactor::new();
        compactor.add_output("output");
        compactor.clear();
        assert!(compactor.is_empty());
    }

    #[test]
    fn test_root_compactor_max_limit() {
        let mut compactor = RootCompactor::new();
        for i in 0..15 {
            compactor.add_output(&format!("output{i}"));
        }
        assert_eq!(compactor.len(), 10);
    }

    #[test]
    fn test_root_compactor_truncation() {
        let mut compactor = RootCompactor::new();
        let long_output = "x".repeat(2000);
        compactor.add_output(&long_output);
        let summary = compactor.get_summary();
        assert!(summary.len() < 2000);
    }
}
