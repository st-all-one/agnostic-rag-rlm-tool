#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::unnecessary_literal_bound
)]

use std::sync::Arc;

use arlm_core::engine::{EngineState, RunNodeParamsOwned, get_forced_solve_reason_owned};
use arlm_core::*;

#[test]
fn test_engine_state_new() {
    let state = EngineState::new();
    assert_eq!(state.nodes_visited(), 0);
    assert_eq!(state.max_depth_seen(), 0);
}

#[test]
fn test_engine_state_next_node_id() {
    let state = EngineState::new();
    let id1 = state.next_node_id();
    let id2 = state.next_node_id();
    assert_eq!(id1, "n1");
    assert_eq!(id2, "n2");
}

#[test]
fn test_engine_state_record_visit() {
    let state = EngineState::new();
    state.record_visit(0);
    state.record_visit(2);
    state.record_visit(1);
    assert_eq!(state.nodes_visited(), 3);
    assert_eq!(state.max_depth_seen(), 2);
}

fn make_test_params(task: &str, depth: u32, max_depth: u32) -> RunNodeParamsOwned {
    let state = Arc::new(EngineState::new());
    let budget = Arc::new(RunBudget::new(1.0, 100_000, 5, 60_000));
    let token_counter = Arc::new(TokenCounter::new(100_000));
    let events = Arc::new(EventBus::new());
    let cache = Arc::new(ResultCache::default());
    let llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync> = Arc::new(MockLlm);
    let input = StartRunInput {
        max_depth,
        max_nodes: 50,
        ..Default::default()
    };
    RunNodeParamsOwned {
        task: task.to_string(),
        depth,
        lineage: Vec::new(),
        parent_id: None,
        input,
        state,
        budget,
        token_counter,
        events,
        cache,
        llm,
        router: Arc::new(parking_lot::Mutex::new(DepthRouter::new())),
        abort: Arc::new(AbortSignal::new()),
        memory: None,
    }
}

#[test]
fn test_get_forced_solve_reason_max_depth() {
    let params = make_test_params("task", 2, 2);
    let reason = get_forced_solve_reason_owned(&params);
    assert!(reason.is_some());
    assert!(reason.unwrap().contains("max depth"));
}

#[test]
fn test_get_forced_solve_reason_cycle() {
    let state = Arc::new(EngineState::new());
    let budget = Arc::new(RunBudget::new(1.0, 100_000, 5, 60_000));
    let token_counter = Arc::new(TokenCounter::new(100_000));
    let events = Arc::new(EventBus::new());
    let cache = Arc::new(ResultCache::default());
    let llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync> = Arc::new(MockLlm);
    let input = StartRunInput::default();
    let params = RunNodeParamsOwned {
        task: "task A".to_string(),
        depth: 0,
        lineage: vec!["task a".to_string()],
        parent_id: None,
        input,
        state,
        budget,
        token_counter,
        events,
        cache,
        llm,
        router: Arc::new(parking_lot::Mutex::new(DepthRouter::new())),
        abort: Arc::new(AbortSignal::new()),
        memory: None,
    };
    let reason = get_forced_solve_reason_owned(&params);
    assert!(reason.is_some());
    assert!(reason.unwrap().contains("cycle"));
}

#[test]
fn test_get_forced_solve_reason_no_forcing() {
    let params = make_test_params("task", 0, 3);
    let reason = get_forced_solve_reason_owned(&params);
    assert!(reason.is_none());
}

struct MockLlm;

#[async_trait::async_trait]
impl arlm_llm::LlmBackend for MockLlm {
    async fn complete(
        &self,
        _req: arlm_llm::CompletionRequest,
    ) -> std::result::Result<arlm_llm::CompletionResponse, arlm_llm::LlmError> {
        Ok(arlm_llm::CompletionResponse {
            content: r#"{"action": "solve", "reason": "mock"}"#.to_string(),
            model: "mock".to_string(),
            usage: arlm_llm::UsageSummary::default(),
        })
    }
    fn name(&self) -> &str {
        "mock"
    }
    async fn health_check(&self) -> std::result::Result<(), arlm_llm::LlmError> {
        Ok(())
    }
}

#[tokio::test]
async fn test_run_rlm_engine_mock() {
    let input = StartRunInput {
        run_id: Arc::from("test-run"),
        task: "test task".to_string(),
        max_depth: 1,
        max_nodes: 10,
        ..Default::default()
    };
    let llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync> = Arc::new(MockLlm);
    let result = run_rlm_engine(input, llm).await.expect("should succeed");
    assert_eq!(result.run_id, "test-run");
    assert!(!result.final_output.is_empty());
    assert!(result.stats.nodes_visited > 0);
}

#[tokio::test]
async fn test_run_rlm_engine_with_memory_persists() {
    #[derive(Default)]
    struct MockMemory {
        saved: std::sync::Mutex<usize>,
    }
    impl arlm_core::MemoryProvider for MockMemory {
        fn context(&self, _task: &str) -> Result<Vec<String>, String> {
            Ok(vec!["ctx".to_string()])
        }
        fn save_trajectory(
            &self,
            _input: &StartRunInput,
            _result: &RlmRunResult,
        ) -> Result<(), String> {
            *self.saved.lock().unwrap() += 1;
            Ok(())
        }
    }

    let input = StartRunInput {
        run_id: Arc::from("test-run-mem"),
        task: "task with memory".to_string(),
        max_depth: 1,
        max_nodes: 10,
        ..Default::default()
    };
    let llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync> = Arc::new(MockLlm);
    let memory: Option<Arc<dyn arlm_core::MemoryProvider>> = Some(Arc::new(MockMemory::default()));
    let result = run_rlm_engine_with_events(input, llm, EventBus::new(), memory)
        .await
        .expect("ok");
    assert_eq!(result.run_id, "test-run-mem");
}

#[test]
fn test_root_compactor_summary_without_llm() {
    let mut compactor = RootCompactor::new();
    assert!(compactor.is_empty());
    compactor.add_output("first output text");
    compactor.add_output("second output text");
    assert_eq!(compactor.len(), 2);
    let summary = compactor.get_summary();
    assert!(summary.contains("first output"));
    assert!(summary.contains("second output"));
}

#[tokio::test]
async fn test_root_compactor_summarize_with_llm() {
    let mut compactor = RootCompactor::new();
    compactor.add_output("accumulated root output A");
    compactor.add_output("accumulated root output B");
    let llm: Arc<dyn arlm_llm::LlmBackend + Send + Sync> = Arc::new(MockLlm);
    let summary = compactor
        .summarize_with_llm(&llm, "gpt-4o", &arlm_llm::RetryConfig::default())
        .await
        .expect("summarize");
    assert!(summary.contains("[Root summary]"));
}
