use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::types::NodeStatus;

/// Events emitted by the RLM engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RlmEvent {
    RunStart {
        run_id: Arc<str>,
        task: String,
        backend: String,
        mode: String,
        max_depth: u32,
        max_nodes: u32,
        max_budget: f64,
        started_at_ms: u64,
    },
    NodeStart {
        run_id: Arc<str>,
        node_id: String,
        depth: u32,
        task: String,
        parent_id: Option<String>,
    },
    NodePlan {
        run_id: Arc<str>,
        node_id: String,
        action: String,
        reason: String,
        subtasks: Vec<String>,
    },
    NodeSolve {
        run_id: Arc<str>,
        node_id: String,
        model: String,
        forced_reason: Option<String>,
    },
    NodeSynthesize {
        run_id: Arc<str>,
        node_id: String,
        model: String,
        children_count: usize,
        compacted: bool,
    },
    CostUpdate {
        run_id: Arc<str>,
        spent: f64,
        budget: f64,
    },
    CacheHit {
        run_id: Arc<str>,
        node_id: String,
        task_hash: Arc<str>,
    },
    NodeEnd {
        run_id: Arc<str>,
        node_id: String,
        status: NodeStatus,
        duration_ms: u64,
        cost: f64,
    },
    RunEnd {
        run_id: Arc<str>,
        duration_ms: u64,
        nodes_visited: u32,
    },
}

const BROADCAST_CAPACITY: usize = 256;

/// Event bus for broadcasting RLM events to subscribers.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RlmEvent>,
}

impl EventBus {
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx }
    }

    pub fn emit(&self, event: RlmEvent) {
        if self.tx.receiver_count() > 0 {
            if let Err(e) = self.tx.send(event) {
                tracing::warn!("event broadcast error: {}", e);
            }
        }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<RlmEvent> {
        self.tx.subscribe()
    }

    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Thread-safe wrapper around [`EventBus`] for ergonomic event emission.
///
/// This is an additive, convenience type: it owns a shared `Arc<EventBus>` and exposes a
/// single `emit` method. It is `Clone` cheaply (just an `Arc` clone) so it can be passed
/// around the engine without touching the raw bus. The underlying `EventBus` API is
/// preserved unchanged.
#[derive(Debug, Clone)]
pub struct EventSink {
    bus: Arc<EventBus>,
}

impl EventSink {
    /// Wrap an existing event bus.
    #[must_use]
    pub fn new(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }

    /// Emit an event on the underlying bus.
    pub fn emit(&self, event: RlmEvent) {
        self.bus.emit(event);
    }

    /// Subscribe to events (delegates to the underlying bus).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<RlmEvent> {
        self.bus.subscribe()
    }

    /// Number of active subscribers (delegates to the underlying bus).
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.bus.subscriber_count()
    }

    /// Access the inner bus.
    #[must_use]
    pub fn bus(&self) -> &Arc<EventBus> {
        &self.bus
    }
}

impl From<EventBus> for EventSink {
    fn from(bus: EventBus) -> Self {
        Self::new(Arc::new(bus))
    }
}

impl From<Arc<EventBus>> for EventSink {
    fn from(bus: Arc<EventBus>) -> Self {
        Self { bus }
    }
}

