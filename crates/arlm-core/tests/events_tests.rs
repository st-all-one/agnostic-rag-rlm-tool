#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::*;
use std::sync::Arc;

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

#[test]
fn test_event_sink_emit() {
    let bus = EventBus::new();
    let sink = arlm_core::EventSink::new(std::sync::Arc::new(bus));
    let mut rx = sink.subscribe();
    sink.emit(RlmEvent::RunStart {
        run_id: Arc::from("run-sink"),
        task: "t".to_string(),
        backend: "openai".to_string(),
        mode: "auto".to_string(),
        max_depth: 1,
        max_nodes: 1,
        max_budget: 1.0,
        started_at_ms: 0,
    });
    let event = rx.try_recv().expect("should receive via sink");
    match event {
        RlmEvent::RunStart { run_id, .. } => assert_eq!(run_id.as_ref(), "run-sink"),
        _ => panic!("expected RunStart"),
    }
}
