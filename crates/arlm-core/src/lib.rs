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
pub mod docker;
pub mod engine;
pub mod events;
pub mod guardrails;
pub mod jsonl_logger;
pub mod logging;
pub mod memory;
pub mod planner;
pub mod repl;
pub mod router;
pub mod sampling;
pub mod solver;
pub mod synthesizer;
pub mod token_counter;
pub mod tools;
pub mod types;

pub use budget::{BudgetSummary, RunBudget};
pub use cache::ResultCache;
pub use compaction::{Compaction, SearchResult as CompactSearchResult};
pub use concurrency::map_concurrent;
pub use docker::{DockerConfig, DockerExecutor, DockerResult};
pub use engine::{EngineState, RootCompactor, run_rlm_engine, run_rlm_engine_with_events};
pub use events::{EventBus, EventSink, RlmEvent};
pub use guardrails::{detect_cycle, normalize_task, sanitize_subtasks};
pub use memory::MemoryProvider;
pub use planner::{parse_planner_decision, plan_node};
pub use repl::{CodeBlock, CodeExecutor, LlmCallback, LlmQueryServer, ReplResult, find_code_blocks, format_repl_result};
pub use router::DepthRouter;
pub use sampling::SamplingArgs;
pub use solver::{PersistentSolver, StateInspector, solve_task};
pub use synthesizer::{build_children_block, synthesize};
pub use token_counter::TokenCounter;
pub use tools::{CodeSearch, ExecutableTool, ListFilesTool, ReadFileTool, SearchCodeTool, ToolRegistry};
pub use types::*;

#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
