use std::sync::Arc;

use tokio::sync::broadcast;

/// Events emitted by the RLM engine.
#[derive(Debug, Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_emit_and_receive() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.emit(RlmEvent::RunStart {
            run_id: Arc::from("run-1"),
            task: "test".to_string(),
            backend: "openai".to_string(),
            mode: "auto".to_string(),
            max_depth: 3,
            max_nodes: 50,
            max_budget: 1.0,
            started_at_ms: 0,
        });

        let event = rx.recv().await.expect("should receive event");
        match event {
            RlmEvent::RunStart { run_id, task, .. } => {
                assert_eq!(run_id.as_ref(), "run-1");
                assert_eq!(task, "test");
            }
            _ => panic!("expected RunStart event"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.emit(RlmEvent::CostUpdate {
            run_id: Arc::from("run-1"),
            spent: 0.5,
            budget: 1.0,
        });

        let e1 = rx1.recv().await.expect("rx1 should receive");
        let e2 = rx2.recv().await.expect("rx2 should receive");

        match (e1, e2) {
            (RlmEvent::CostUpdate { spent: s1, .. }, RlmEvent::CostUpdate { spent: s2, .. }) => {
                assert!((s1 - 0.5).abs() < f64::EPSILON);
                assert!((s2 - 0.5).abs() < f64::EPSILON);
            }
            _ => panic!("expected CostUpdate events"),
        }
    }

    #[test]
    fn test_event_bus_subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);
        let _rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn test_event_bus_emit_no_subscribers() {
        let bus = EventBus::new();
        bus.emit(RlmEvent::CostUpdate {
            run_id: Arc::from("run-1"),
            spent: 0.0,
            budget: 1.0,
        });
    }
}
