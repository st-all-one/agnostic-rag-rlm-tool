#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
pub mod budget;
pub mod cache;
pub mod compaction;
pub mod concurrency;
pub mod engine;
pub mod events;
pub mod guardrails;
pub mod jsonl_logger;
pub mod logging;
pub mod planner;
pub mod router;
pub mod sampling;
pub mod solver;
pub mod synthesizer;
pub mod token_counter;
pub mod types;

pub use budget::{BudgetSummary, RunBudget};
pub use cache::ResultCache;
pub use compaction::{Compaction, SearchResult as CompactSearchResult};
pub use concurrency::map_concurrent;
pub use engine::{EngineState, run_rlm_engine, run_rlm_engine_with_events};
pub use events::{EventBus, RlmEvent};
pub use guardrails::{detect_cycle, normalize_task, sanitize_subtasks};
pub use planner::{parse_planner_decision, plan_node};
pub use router::DepthRouter;
pub use solver::solve_task;
pub use synthesizer::{build_children_block, synthesize};
pub use token_counter::TokenCounter;
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
