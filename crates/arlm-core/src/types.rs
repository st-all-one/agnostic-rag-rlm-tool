use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use arlm_llm::RetryConfig;

/// Backend kind for the RLM run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RlmBackend {
    OpenAi,
    Anthropic,
    Gemini,
    Ollama,
}

impl fmt::Display for RlmBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Gemini => write!(f, "gemini"),
            Self::Ollama => write!(f, "ollama"),
        }
    }
}

/// RLM execution mode.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RlmMode {
    #[default]
    Auto,
    Solve,
    Decompose,
}

/// Tools profile to pass to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolsProfile {
    pub name: String,
    pub tools: Vec<String>,
}

impl Default for ToolsProfile {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            tools: Vec::new(),
        }
    }
}

/// Compaction policy for context management.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactionPolicy {
    /// Maximum tokens before compacting child outputs.
    pub max_child_tokens: u32,
    /// Whether compaction is enabled.
    pub enabled: bool,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            max_child_tokens: 8_000,
            enabled: true,
        }
    }
}

/// Input for starting an RLM run.
#[derive(Debug, Clone)]
pub struct StartRunInput {
    pub run_id: Arc<str>,
    pub task: String,
    pub backend: RlmBackend,
    pub mode: RlmMode,
    pub model: Option<String>,
    pub project: String,
    pub tools_profile: ToolsProfile,
    pub max_depth: u32,
    pub max_nodes: u32,
    pub max_branching: u32,
    pub concurrency: usize,
    pub timeout_ms: u64,
    pub max_budget: f64,
    pub max_tokens: u64,
    pub max_errors: u32,
    pub agent: String,
    pub retry_policy: RetryPolicy,
    pub enable_cache: bool,
    pub compaction: CompactionPolicy,
}

impl Default for StartRunInput {
    fn default() -> Self {
        Self {
            run_id: Arc::from(""),
            task: String::new(),
            backend: RlmBackend::OpenAi,
            mode: RlmMode::Auto,
            model: None,
            project: String::new(),
            tools_profile: ToolsProfile::default(),
            max_depth: 3,
            max_nodes: 50,
            max_branching: 4,
            concurrency: 4,
            timeout_ms: 300_000,
            max_budget: 1.0,
            max_tokens: 100_000,
            max_errors: 5,
            agent: "arlm".to_string(),
            retry_policy: RetryPolicy::default(),
            enable_cache: true,
            compaction: CompactionPolicy::default(),
        }
    }
}

/// Retry policy wrapping arlm-llm's `RetryConfig`.
#[derive(Debug, Clone, Default)]
pub struct RetryPolicy {
    pub inner: RetryConfig,
}

impl RetryPolicy {
    #[must_use]
    pub fn new(max_retries: u32, base_delay_ms: u64, max_delay_ms: u64) -> Self {
        Self {
            inner: RetryConfig::new(max_retries, base_delay_ms, max_delay_ms),
        }
    }
}

/// Node status in the RLM tree.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Skipped,
    Cancelled,
    Cached,
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Cached => write!(f, "cached"),
        }
    }
}

/// Action the planner decides on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Solve,
    Decompose,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Solve => write!(f, "solve"),
            Self::Decompose => write!(f, "decompose"),
        }
    }
}

/// Planner decision output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerDecision {
    pub action: Action,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtasks: Option<Vec<String>>,
}

impl Default for PlannerDecision {
    fn default() -> Self {
        Self {
            action: Action::Solve,
            reason: String::new(),
            subtasks: None,
        }
    }
}

/// Run statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunStats {
    pub nodes_visited: u32,
    pub max_depth_seen: u32,
    pub duration_ms: u64,
}

/// Final result of an RLM run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmRunResult {
    pub run_id: String,
    pub backend: String,
    pub final_output: String,
    pub root: RlmNode,
    pub stats: RunStats,
}

/// Abort signal for cancelling runs.
#[derive(Debug, Clone)]
pub struct AbortSignal {
    cancelled: Arc<AtomicBool>,
}

impl AbortSignal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

/// Usage summary accumulated across nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeUsage {
    pub cost_usd: f64,
    pub tokens: u32,
    pub errors: u32,
}

/// A node in the RLM decision tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmNode {
    pub id: String,
    pub depth: u32,
    pub task: String,
    pub status: NodeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<PlannerDecision>,
    pub started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub children: Vec<RlmNode>,
    #[serde(default)]
    pub usage: NodeUsage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partial_answer: Option<String>,
    #[serde(default)]
    pub cached: bool,
}

impl RlmNode {
    #[must_use]
    pub fn running(id: &str, depth: u32, task: &str) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Running,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: None,
            result: None,
            error: None,
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: false,
        }
    }

    #[must_use]
    pub fn completed(id: &str, depth: u32, task: &str, result: String) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Completed,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            result: Some(result),
            error: None,
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: false,
        }
    }

    #[must_use]
    pub fn failed(id: &str, depth: u32, task: &str, error: String) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Failed,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            result: None,
            error: Some(error),
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: false,
        }
    }

    #[must_use]
    pub fn skipped(id: &str, depth: u32, task: &str) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Skipped,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            result: None,
            error: Some("budget exhausted".to_string()),
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: false,
        }
    }

    #[must_use]
    pub fn cancelled(id: &str, depth: u32, task: &str) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Cancelled,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            result: None,
            error: Some("cancelled".to_string()),
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: false,
        }
    }

    #[must_use]
    pub fn cached(id: &str, depth: u32, task: &str, result: String) -> Self {
        Self {
            id: id.to_string(),
            depth,
            task: task.to_string(),
            status: NodeStatus::Cached,
            decision: None,
            started_at_ms: now_ms(),
            finished_at_ms: Some(now_ms()),
            result: Some(result),
            error: None,
            children: Vec::new(),
            usage: NodeUsage::default(),
            partial_answer: None,
            cached: true,
        }
    }

    #[must_use]
    pub fn with_children(mut self, children: Vec<RlmNode>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub fn with_decision(mut self, decision: PlannerDecision) -> Self {
        self.decision = Some(decision);
        self
    }

    pub fn finish(&mut self, result: Option<String>, error: Option<String>) {
        self.finished_at_ms = Some(now_ms());
        if let Some(r) = result {
            self.result = Some(r);
            self.status = NodeStatus::Completed;
        }
        if let Some(e) = error {
            self.error = Some(e);
            self.status = NodeStatus::Failed;
        }
    }

    #[must_use]
    pub fn total_usage(&self) -> NodeUsage {
        let mut usage = self.usage.clone();
        for child in &self.children {
            let child_usage = child.total_usage();
            usage.cost_usd += child_usage.cost_usd;
            usage.tokens += child_usage.tokens;
            usage.errors += child_usage.errors;
        }
        usage
    }
}

/// Get current time in milliseconds since epoch.
#[must_use]
pub fn now_ms() -> u64 {
    #[allow(clippy::cast_sign_loss)]
    let ts = chrono::Utc::now().timestamp_millis() as u64;
    ts
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]
mod tests {
    use super::*;

    #[test]
    fn test_rlm_backend_display() {
        assert_eq!(RlmBackend::OpenAi.to_string(), "openai");
        assert_eq!(RlmBackend::Anthropic.to_string(), "anthropic");
    }

    #[test]
    fn test_node_status_display() {
        assert_eq!(NodeStatus::Running.to_string(), "running");
        assert_eq!(NodeStatus::Completed.to_string(), "completed");
    }

    #[test]
    fn test_action_display() {
        assert_eq!(Action::Solve.to_string(), "solve");
        assert_eq!(Action::Decompose.to_string(), "decompose");
    }

    #[test]
    fn test_planner_decision_default() {
        let d = PlannerDecision::default();
        assert_eq!(d.action, Action::Solve);
        assert!(d.subtasks.is_none());
    }

    #[test]
    fn test_rlm_node_running() {
        let node = RlmNode::running("n1", 0, "test task");
        assert_eq!(node.id, "n1");
        assert_eq!(node.depth, 0);
        assert_eq!(node.status, NodeStatus::Running);
        assert!(node.result.is_none());
    }

    #[test]
    fn test_rlm_node_completed() {
        let node = RlmNode::completed("n2", 1, "task", "result".to_string());
        assert_eq!(node.status, NodeStatus::Completed);
        assert_eq!(node.result.as_deref(), Some("result"));
    }

    #[test]
    fn test_rlm_node_failed() {
        let node = RlmNode::failed("n3", 2, "task", "oops".to_string());
        assert_eq!(node.status, NodeStatus::Failed);
        assert_eq!(node.error.as_deref(), Some("oops"));
    }

    #[test]
    fn test_rlm_node_skipped() {
        let node = RlmNode::skipped("n4", 0, "task");
        assert_eq!(node.status, NodeStatus::Skipped);
    }

    #[test]
    fn test_rlm_node_cancelled() {
        let node = RlmNode::cancelled("n5", 0, "task");
        assert_eq!(node.status, NodeStatus::Cancelled);
    }

    #[test]
    fn test_rlm_node_cached() {
        let node = RlmNode::cached("n6", 0, "task", "cached result".to_string());
        assert_eq!(node.status, NodeStatus::Cached);
        assert!(node.cached);
    }

    #[test]
    fn test_rlm_node_with_children() {
        let child = RlmNode::completed("c1", 1, "child", "ok".to_string());
        let parent = RlmNode::running("p1", 0, "parent").with_children(vec![child]);
        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].id, "c1");
    }

    #[test]
    fn test_rlm_node_with_decision() {
        let decision = PlannerDecision {
            action: Action::Decompose,
            reason: "complex task".to_string(),
            subtasks: Some(vec!["a".to_string(), "b".to_string()]),
        };
        let node = RlmNode::running("n1", 0, "task").with_decision(decision);
        let d = node.decision.as_ref().expect("decision should exist");
        assert_eq!(d.action, Action::Decompose);
    }

    #[test]
    fn test_rlm_node_finish() {
        let mut node = RlmNode::running("n1", 0, "task");
        node.finish(Some("done".to_string()), None);
        assert_eq!(node.status, NodeStatus::Completed);
        assert!(node.finished_at_ms.is_some());
    }

    #[test]
    fn test_rlm_node_finish_with_error() {
        let mut node = RlmNode::running("n1", 0, "task");
        node.finish(None, Some("error".to_string()));
        assert_eq!(node.status, NodeStatus::Failed);
    }

    #[test]
    fn test_rlm_node_total_usage_empty() {
        let node = RlmNode::running("n1", 0, "task");
        let usage = node.total_usage();
        assert_eq!(usage.cost_usd, 0.0);
        assert_eq!(usage.tokens, 0);
    }

    #[test]
    fn test_rlm_node_total_usage_with_children() {
        let mut child = RlmNode::completed("c1", 1, "child", "ok".to_string());
        child.usage = NodeUsage {
            cost_usd: 0.1,
            tokens: 100,
            errors: 0,
        };
        let mut parent = RlmNode::running("p1", 0, "parent");
        parent.usage = NodeUsage {
            cost_usd: 0.05,
            tokens: 50,
            errors: 0,
        };
        parent.children = vec![child];
        let total = parent.total_usage();
        assert!((total.cost_usd - 0.15).abs() < f64::EPSILON);
        assert_eq!(total.tokens, 150);
    }

    #[test]
    fn test_abort_signal() {
        let signal = AbortSignal::new();
        assert!(!signal.is_cancelled());
        signal.cancel();
        assert!(signal.is_cancelled());
    }

    #[test]
    fn test_now_ms() {
        let ms = now_ms();
        assert!(ms > 0);
    }

    #[test]
    fn test_start_run_input_default() {
        let input = StartRunInput::default();
        assert_eq!(input.max_depth, 3);
        assert_eq!(input.max_nodes, 50);
        assert_eq!(input.concurrency, 4);
    }

    #[test]
    fn test_compaction_policy_default() {
        let p = CompactionPolicy::default();
        assert!(p.enabled);
        assert_eq!(p.max_child_tokens, 8_000);
    }

    #[test]
    fn test_rlm_run_result_serialization() {
        let result = RlmRunResult {
            run_id: "test".to_string(),
            backend: "openai".to_string(),
            final_output: "output".to_string(),
            root: RlmNode::completed("n1", 0, "task", "result".to_string()),
            stats: RunStats::default(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("test"));
    }
}
