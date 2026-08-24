//! Domain types for `arlm-core`.
//!
//! The recursive-RLM engine types (`RlmNode`, `StartRunInput`, `RlmRunResult`,
//! `RlmBackend`, `CompactionPolicy`, …) were removed in plan 019. This module
//! is retained as the home for shared domain types used by the surviving
//! `qa_cache` and `memory` modules; it is currently empty after the legacy
//! pruning.

pub mod enums;
pub mod input;
pub mod node;
