use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::token_counter::get_context_limit;

/// Root-level compaction threshold: trigger when context reaches 60% of model limit.
const ROOT_COMPACTION_THRESHOLD: f64 = 0.60;

/// Shared engine state with atomic counters.
///
/// All counters are `Atomic*` so the engine stays lock-free and thread-safe while
/// many node tasks run concurrently.
#[derive(Debug)]
pub struct EngineState {
    nodes_visited: AtomicU32,
    max_depth_seen: AtomicU32,
    next_id: AtomicU64,
    total_output_tokens: AtomicU32,
}

impl EngineState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nodes_visited: AtomicU32::new(0),
            max_depth_seen: AtomicU32::new(0),
            next_id: AtomicU64::new(1),
            total_output_tokens: AtomicU32::new(0),
        }
    }

    #[must_use]
    pub fn next_node_id(&self) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("n{id}")
    }

    pub fn record_visit(&self, depth: u32) {
        self.nodes_visited.fetch_add(1, Ordering::Relaxed);
        self.max_depth_seen.fetch_max(depth, Ordering::Relaxed);
    }

    /// Record output tokens from a node.
    pub fn record_output_tokens(&self, tokens: u32) {
        self.total_output_tokens
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// Check if root-level compaction is needed.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn needs_root_compaction(&self, model: &str) -> bool {
        let model_limit = get_context_limit(model);
        let threshold = (f64::from(model_limit) * ROOT_COMPACTION_THRESHOLD) as u32;
        let total = self.total_output_tokens.load(Ordering::Relaxed);
        total >= threshold
    }

    #[must_use]
    pub fn nodes_visited(&self) -> u32 {
        self.nodes_visited.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn max_depth_seen(&self) -> u32 {
        self.max_depth_seen.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn total_output_tokens(&self) -> u32 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }
}

impl Default for EngineState {
    fn default() -> Self {
        Self::new()
    }
}
