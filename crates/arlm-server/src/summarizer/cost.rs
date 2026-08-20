/// Cost estimation for summarization operations.
#[derive(Debug, Clone)]
pub struct CostEstimate {
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Number of LLM calls required.
    pub llm_calls: u32,
    /// Estimated duration in seconds.
    pub duration_seconds: f64,
}

/// Estimate the cost of summarizing a project.
///
/// # Arguments
///
/// * `file_count` - Number of files to summarize
/// * `avg_chunks_per_file` - Average chunks per file
/// * `model_cost_per_1k_tokens` - Cost per 1K tokens (default: $0.01 for GPT-4o-mini)
pub fn estimate_cost(
    file_count: u32,
    avg_chunks_per_file: u32,
    model_cost_per_1k_tokens: f64,
) -> CostEstimate {
    // Per-file summarization
    let file_calls = file_count;
    let file_tokens = file_count * avg_chunks_per_file * 800; // ~800 tokens per chunk
    let file_cost = (file_tokens as f64 / 1000.0) * model_cost_per_1k_tokens;

    // Per-module summarization (assume 1 module per 10 files)
    let module_calls = file_count / 10 + 1;
    let module_tokens = module_calls * 3000; // ~3K tokens per module summary input
    let module_cost = (module_tokens as f64 / 1000.0) * model_cost_per_1k_tokens;

    // Per-project summarization
    let project_calls = 1;
    let project_tokens = 5000; // ~5K tokens for project summary
    let project_cost = (project_tokens as f64 / 1000.0) * model_cost_per_1k_tokens;

    let total_cost = file_cost + module_cost + project_cost;
    let total_calls = file_calls + module_calls + project_calls;

    // Estimate duration: ~2 seconds per LLM call
    let duration = total_calls as f64 * 2.0;

    CostEstimate {
        cost_usd: total_cost,
        llm_calls: total_calls,
        duration_seconds: duration,
    }
}

/// Estimate incremental re-summarization cost.
///
/// Only summarizes changed files and their parent modules.
pub fn estimate_incremental_cost(
    changed_files: u32,
    _total_files: u32,
    avg_chunks_per_file: u32,
    model_cost_per_1k_tokens: f64,
) -> CostEstimate {
    // Only changed files need re-summarization
    let file_cost = estimate_cost(changed_files, avg_chunks_per_file, model_cost_per_1k_tokens);

    // Modules containing changed files need re-summarization
    let affected_modules = (changed_files / 10 + 1) as f64;
    let module_tokens = (affected_modules * 3000.0) as u32;
    let module_cost = (module_tokens as f64 / 1000.0) * model_cost_per_1k_tokens;

    // Project summary always needs refresh
    let project_cost = (5000.0 / 1000.0) * model_cost_per_1k_tokens;

    let total_cost = file_cost.cost_usd + module_cost + project_cost;
    let total_calls = file_cost.llm_calls + (affected_modules as u32) + 1;
    let duration = total_calls as f64 * 2.0;

    CostEstimate {
        cost_usd: total_cost,
        llm_calls: total_calls,
        duration_seconds: duration,
    }
}
