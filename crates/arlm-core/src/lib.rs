pub mod budget;
pub mod cache;
pub mod concurrency;
pub mod engine;
pub mod events;
pub mod guardrails;
pub mod logging;
pub mod planner;
pub mod solver;
pub mod synthesizer;
pub mod types;

pub use budget::{BudgetSummary, RunBudget};
pub use cache::ResultCache;
pub use concurrency::map_concurrent;
pub use engine::{EngineState, run_rlm_engine};
pub use events::{EventBus, RlmEvent};
pub use guardrails::{detect_cycle, normalize_task, sanitize_subtasks};
pub use planner::{parse_planner_decision, plan_node};
pub use solver::solve_task;
pub use synthesizer::{build_children_block, synthesize};
pub use types::*;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        assert!(!version().is_empty());
    }
}
