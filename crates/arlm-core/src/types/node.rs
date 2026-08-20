use serde::{Deserialize, Serialize};

use super::enums::{NodeStatus, NodeUsage, PlannerDecision, now_ms};

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
