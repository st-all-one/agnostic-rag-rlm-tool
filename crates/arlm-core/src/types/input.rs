use super::enums::{AbortSignal, CompactionPolicy, RetryPolicy, RlmBackend, RlmMode, ToolsProfile};

/// Input for starting an RLM run.
#[derive(Debug, Clone)]
pub struct StartRunInput {
    pub run_id: std::sync::Arc<str>,
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
    pub abort: AbortSignal,
    pub custom_tools: Vec<super::enums::CustomTool>,
}

impl Default for StartRunInput {
    fn default() -> Self {
        Self {
            run_id: std::sync::Arc::from(""),
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
            abort: AbortSignal::new(),
            custom_tools: Vec::new(),
        }
    }
}
