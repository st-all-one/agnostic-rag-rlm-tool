use std::fmt::{self, Write};
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
    DeepSeek,
    MiMo,
}

impl fmt::Display for RlmBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAi => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Gemini => write!(f, "gemini"),
            Self::Ollama => write!(f, "ollama"),
            Self::DeepSeek => write!(f, "deepseek"),
            Self::MiMo => write!(f, "mimo"),
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
    /// REPL mode: LLM generates code blocks that are executed in a subprocess loop.
    Repl,
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

/// A custom tool available to the RLM solver/planner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomTool {
    /// Tool name (e.g., "`search_code`", "`read_file`").
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// Optional parameter schema description (e.g., "query: str, limit: int = 10").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<String>,
    /// Whether this tool is callable (function) or a data constant.
    #[serde(default = "default_true")]
    pub callable: bool,
}

fn default_true() -> bool {
    true
}

impl CustomTool {
    #[must_use]
    pub fn function(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters: None,
            callable: true,
        }
    }

    #[must_use]
    pub fn with_parameters(mut self, parameters: &str) -> Self {
        self.parameters = Some(parameters.to_string());
        self
    }

    #[must_use]
    pub fn data(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters: None,
            callable: false,
        }
    }
}

/// Format custom tools into a bullet-point string for system prompt injection.
#[must_use]
pub fn format_tools_for_prompt(tools: &[CustomTool]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut out = String::from("Available tools:\n");
    for tool in tools {
        let kind = if tool.callable { "function" } else { "data" };
        if let Some(ref params) = tool.parameters {
            let _ = writeln!(
                out,
                "- {}({}) → {} [{}]",
                tool.name, params, tool.description, kind
            );
        } else {
            let _ = writeln!(out, "- {} → {} [{}]", tool.name, tool.description, kind);
        }
    }
    out
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
    pub root: super::node::RlmNode,
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

/// Get current time in milliseconds since epoch.
#[must_use]
pub fn now_ms() -> u64 {
    #[allow(clippy::cast_sign_loss)]
    let ts = chrono::Utc::now().timestamp_millis() as u64;
    ts
}
