use std::fmt::Write;
use std::sync::Arc;

use anyhow::Result;
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role, retry::retry_with_backoff};
use tracing::info;

use crate::budget::RunBudget;
use crate::cache::ResultCache;
use crate::logging::ScopedTimer;
use crate::memory::MemoryProvider;
use crate::repl::{CodeExecutor, find_code_blocks, format_repl_result};
use crate::token_counter::{TokenCounter, get_context_limit};
use crate::types::{Action, StartRunInput, format_tools_for_prompt};

const SOLVER_SYSTEM: &str = "You are a worker node in an RLM system. Solve the task directly and return a concrete, actionable answer.";

/// Compaction threshold: trigger summarization when context reaches 60% of model limit.
const COMPACTION_THRESHOLD: f64 = 0.60;

/// Maximum number of compaction iterations.
const MAX_COMPACTION_ITERATIONS: u32 = 5;

/// Solve a task directly by calling the LLM.
///
/// Supports iterative refinement with compaction when context exceeds 85% of model limit.
///
/// When `memory` is `Some`, relevant context is fetched from the memory provider and
/// prepended to the system prompt before the LLM call (no-op when `None`).
///
/// # Errors
///
/// Returns an error if the LLM call fails.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn solve_task(
    task: &str,
    input: &StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    cache: &ResultCache,
    memory: Option<Arc<dyn MemoryProvider>>,
    forced_reason: Option<&str>,
    model_override: Option<&str>,
) -> Result<String> {
    let _timer = ScopedTimer::new("solve_task");

    if input.enable_cache {
        if let Some(cached) = cache.get(task, &input.project) {
            info!(task = task, "cache hit");
            return Ok(cached);
        }
    }

    let prompt = if let Some(reason) = forced_reason {
        format!(
            "Solve this task directly. You were forced to solve because: {reason}\n\nTask: {task}\n\nProvide a concrete, actionable answer.",
        )
    } else {
        format!("Solve this task directly and return a concrete answer.\n\nTask: {task}")
    };

    let model = model_override
        .or(input.model.as_deref())
        .unwrap_or("gpt-4o")
        .to_string();

    let tools_block = format_tools_for_prompt(&input.custom_tools);
    let memory_block = build_memory_context(memory.as_ref(), task);
    let system_content = if tools_block.is_empty() && memory_block.is_empty() {
        SOLVER_SYSTEM.to_string()
    } else if memory_block.is_empty() {
        format!("{SOLVER_SYSTEM}\n\n{tools_block}")
    } else if tools_block.is_empty() {
        format!("{SOLVER_SYSTEM}\n\n{memory_block}")
    } else {
        format!("{SOLVER_SYSTEM}\n\n{tools_block}\n\n{memory_block}")
    };

    let mut messages: Vec<Message> = vec![
        Message {
            role: Role::System,
            content: system_content,
        },
        Message {
            role: Role::User,
            content: prompt,
        },
    ];

    let sampling = crate::sampling::SamplingArgs::for_node_type(Action::Solve);
    let model_limit = get_context_limit(&model);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let threshold_tokens = (f64::from(model_limit) * COMPACTION_THRESHOLD) as u32;

    let mut compaction_count = 0u32;

    // Iterative refinement loop with compaction
    for iteration in 0..MAX_COMPACTION_ITERATIONS {
        // Check if compaction is needed
        let current_tokens = estimate_messages_tokens(&messages);
        if current_tokens >= threshold_tokens && iteration > 0 {
            compaction_count += 1;
            info!(
                task = task,
                iteration,
                current_tokens,
                threshold_tokens,
                "compaction triggered"
            );
            messages = compact_messages(&messages, &llm, &input.retry_policy.inner, &model).await?;
        }

        let request = sampling.clone().apply_to_request(CompletionRequest {
            model: model.clone(),
            messages: messages.clone(),
            temperature: None,
            max_tokens: Some(2048),
            stop: None,
        });

        let response = retry_with_backoff(&input.retry_policy.inner, || {
            let req = request.clone();
            let llm = llm.clone();
            async move { llm.complete(req).await }
        })
        .await?;

        budget.record_call(&model, &response.usage);

        // Check if the response contains a final answer (no more code blocks)
        let blocks = find_code_blocks(&response.content);
        if blocks.is_empty() || iteration == MAX_COMPACTION_ITERATIONS - 1 {
            if input.enable_cache {
                cache.put(task, &input.project, &response.content);
            }

            info!(
                task = task,
                iterations = iteration + 1,
                compaction_count,
                tokens = response.usage.total_tokens,
                "task solved"
            );

            return Ok(response.content);
        }

        // Execute code blocks and append results
        let executor = CodeExecutor::default_executor();
        messages.push(Message {
            role: Role::Assistant,
            content: response.content.clone(),
        });

        let mut results_text = String::new();
        for block in &blocks {
            match executor.execute(block) {
                Ok(result) => {
                    results_text.push_str(&format_repl_result(&result));
                    results_text.push('\n');
                }
                Err(e) => {
                    results_text.push_str("[execution error: ");
                    results_text.push_str(&e);
                    results_text.push_str("]\n");
                }
            }
        }

        messages.push(Message {
            role: Role::User,
            content: format!("Code executed:\n{results_text}\nAnalyze the output and either write more code or provide your final answer."),
        });
    }

    // Exhausted iterations — ask for final answer
    messages.push(Message {
        role: Role::User,
        content: "You have reached the maximum number of iterations. Provide your final answer now based on all the work done so far.".to_string(),
    });

    let final_request = sampling.apply_to_request(CompletionRequest {
        model: model.clone(),
        messages,
        temperature: None,
        max_tokens: Some(2048),
        stop: None,
    });

    let final_response = retry_with_backoff(&input.retry_policy.inner, || {
        let req = final_request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(&model, &final_response.usage);

    if input.enable_cache {
        cache.put(task, &input.project, &final_response.content);
    }

    info!(
        task = task,
        iterations = MAX_COMPACTION_ITERATIONS,
        compaction_count,
        "task solved (max iterations)"
    );

    Ok(final_response.content)
}

/// Estimate total tokens in a message list.
fn estimate_messages_tokens(messages: &[Message]) -> u32 {
    messages
        .iter()
        .map(|m| TokenCounter::estimate(&m.content))
        .sum()
}

/// Compact messages by summarizing older content.
///
/// Keeps the system message and most recent messages, summarizes older ones.
async fn compact_messages(
    messages: &[Message],
    llm: &Arc<dyn LlmBackend + Send + Sync>,
    retry_config: &arlm_llm::RetryConfig,
    model: &str,
) -> Result<Vec<Message>> {
    if messages.len() <= 2 {
        return Ok(messages.to_vec());
    }

    // Keep system message (index 0) and last 4 messages
    let system_msg = &messages[0];
    let recent_start = messages.len().saturating_sub(4);
    let old_messages = &messages[1..recent_start];
    let recent_messages = &messages[recent_start..];

    // Build summarization prompt
    let old_content: Vec<String> = old_messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "System",
                Role::User => "User",
                Role::Assistant => "Assistant",
            };
            format!("{role}: {}", truncate(&m.content, 2000))
        })
        .collect();

    let summary_prompt = format!(
        "Summarize the following conversation history concisely. Preserve key results, decisions, and context. Be brief but complete.\n\n{}",
        old_content.join("\n\n")
    );

    let summary_messages = vec![
        Message {
            role: Role::System,
            content: "You are a conversation summarizer. Produce a concise summary preserving key information.".to_string(),
        },
        Message {
            role: Role::User,
            content: summary_prompt,
        },
    ];

    let request = CompletionRequest {
        model: model.to_string(),
        messages: summary_messages,
        temperature: Some(0.3),
        max_tokens: Some(1024),
        stop: None,
    };

    let response = retry_with_backoff(retry_config, || {
        let req = request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    // Build new message list: system + summary + recent
    let mut new_messages = Vec::with_capacity(2 + recent_messages.len());
    new_messages.push(system_msg.clone());
    new_messages.push(Message {
        role: Role::Assistant,
        content: format!(
            "[Context compacted {} time(s)] Summary of previous work:\n{}",
            1, response.content
        ),
    });
    new_messages.extend_from_slice(recent_messages);

    Ok(new_messages)
}

/// Truncate text to `max_chars`, adding "..." if truncated.
fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!("{}...", &text[..max_chars])
    }
}

/// Build a memory-context block from the provider, or an empty string when unavailable.
fn build_memory_context(memory: Option<&Arc<dyn MemoryProvider>>, task: &str) -> String {
    let Some(provider) = memory else {
        return String::new();
    };
    match provider.context(task) {
        Ok(context) if !context.is_empty() => {
            let joined = context
                .iter()
                .filter(|c| !c.trim().is_empty())
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                String::new()
            } else {
                format!("Relevant memory context:\n{joined}")
            }
        }
        Ok(_) => String::new(),
        Err(e) => {
            tracing::warn!(error = %e, "memory context fetch failed; continuing without it");
            String::new()
        }
    }
}

/// Persistent solver that maintains conversation history across calls.
///
/// Enables multi-turn conversations where the solver remembers previous context.
pub struct PersistentSolver {
    history: Vec<Message>,
    max_history_tokens: u32,
}

impl PersistentSolver {
    /// Create a new persistent solver.
    #[must_use]
    pub fn new(max_history_tokens: u32) -> Self {
        Self {
            history: Vec::new(),
            max_history_tokens,
        }
    }

    /// Add a user message to history.
    pub fn add_user_message(&mut self, content: &str) {
        self.history.push(Message {
            role: Role::User,
            content: content.to_string(),
        });
    }

    /// Add an assistant message to history.
    pub fn add_assistant_message(&mut self, content: &str) {
        self.history.push(Message {
            role: Role::Assistant,
            content: content.to_string(),
        });
    }

    /// Get the conversation history.
    #[must_use]
    pub fn history(&self) -> &[Message] {
        &self.history
    }

    /// Clear the conversation history.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Compact history if it exceeds the token limit.
    pub fn compact_if_needed(&mut self) {
        let total_tokens: u32 = self
            .history
            .iter()
            .map(|m| TokenCounter::estimate(&m.content))
            .sum();

        if total_tokens > self.max_history_tokens {
            // Keep the last 4 messages, discard the rest
            let keep = self.history.len().min(4);
            let drain = self.history.len() - keep;
            self.history.drain(..drain);

            tracing::info!(
                dropped = drain,
                remaining = keep,
                "persistent solver history compacted"
            );
        }
    }

    /// Solve a task with conversation history context.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM call fails.
    pub async fn solve_with_history(
        &mut self,
        task: &str,
        input: &StartRunInput,
        llm: &Arc<dyn LlmBackend + Send + Sync>,
        budget: &RunBudget,
    ) -> Result<String> {
        // Compact history if needed
        self.compact_if_needed();

        let model = input
            .model
            .as_deref()
            .unwrap_or("gpt-4o")
            .to_string();

        let tools_block = format_tools_for_prompt(&input.custom_tools);
        let system_content = if tools_block.is_empty() {
            SOLVER_SYSTEM.to_string()
        } else {
            format!("{SOLVER_SYSTEM}\n\n{tools_block}")
        };

        let mut messages = vec![Message {
            role: Role::System,
            content: system_content,
        }];

        // Add conversation history
        messages.extend_from_slice(&self.history);

        // Add current task
        messages.push(Message {
            role: Role::User,
            content: format!("Task: {task}\n\nSolve this task directly and return a concrete answer."),
        });

        let sampling = crate::sampling::SamplingArgs::for_node_type(Action::Solve);
        let request = sampling.apply_to_request(CompletionRequest {
            model: model.clone(),
            messages,
            temperature: None,
            max_tokens: Some(2048),
            stop: None,
        });

        let response = retry_with_backoff(&input.retry_policy.inner, || {
            let req = request.clone();
            let llm = llm.clone();
            async move { llm.complete(req).await }
        })
        .await?;

        budget.record_call(&model, &response.usage);

        // Add to history
        self.add_user_message(&format!("Task: {task}"));
        self.add_assistant_message(&response.content);

        Ok(response.content)
    }
}

/// State inspector for the solver.
///
/// Provides introspection capabilities to see what has been done.
pub struct StateInspector {
    completed_tasks: Vec<String>,
    current_variables: std::collections::HashMap<String, String>,
    iteration_count: u32,
}

impl StateInspector {
    /// Create a new state inspector.
    #[must_use]
    pub fn new() -> Self {
        Self {
            completed_tasks: Vec::new(),
            current_variables: std::collections::HashMap::new(),
            iteration_count: 0,
        }
    }

    /// Record a completed task.
    pub fn record_task(&mut self, task: &str, result: &str) {
        self.completed_tasks.push(task.to_string());
        self.current_variables
            .insert(format!("task_{}", self.completed_tasks.len()), result.to_string());
        self.iteration_count += 1;
    }

    /// Set a variable.
    pub fn set_variable(&mut self, name: &str, value: &str) {
        self.current_variables
            .insert(name.to_string(), value.to_string());
    }

    /// Get a variable.
    #[must_use]
    pub fn get_variable(&self, name: &str) -> Option<&str> {
        self.current_variables.get(name).map(std::string::String::as_str)
    }

    /// List all variables (equivalent to `SHOW_VARS`).
    #[must_use]
    pub fn show_vars(&self) -> String {
        if self.current_variables.is_empty() {
            return "No variables created yet.".to_string();
        }

        let mut output = String::from("Available variables:\n");
        for (name, value) in &self.current_variables {
            let preview = if value.len() > 50 {
                format!("{}...", &value[..50])
            } else {
                value.clone()
            };
            let _ = writeln!(output, "  {name}: {preview}");
        }
        output
    }

    /// Get a summary of completed work.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Completed {} tasks. {} variables set.",
            self.completed_tasks.len(),
            self.current_variables.len()
        )
    }

    /// Get the iteration count.
    #[must_use]
    pub fn iteration_count(&self) -> u32 {
        self.iteration_count
    }

    /// Get all completed tasks.
    #[must_use]
    pub fn completed_tasks(&self) -> &[String] {
        &self.completed_tasks
    }
}

impl Default for StateInspector {
    fn default() -> Self {
        Self::new()
    }
}

const REPL_SYSTEM: &str = "You are a coding assistant in REPL mode. Write code in ```python or ```bash blocks to solve the task. After executing code, analyze the output and either write more code or provide a final answer. When the task is complete, provide your final answer as plain text (no more code blocks).";

const MAX_REPL_ITERATIONS: u32 = 10;

/// Solve a task using REPL mode: LLM generates code, code is executed, results feed back.
///
/// When `memory` is `Some`, relevant context is prepended to the system prompt (no-op when `None`).
///
/// # Errors
///
/// Returns an error if the LLM call fails.
pub async fn solve_task_repl(
    task: &str,
    input: &StartRunInput,
    llm: Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    cache: &ResultCache,
    memory: Option<Arc<dyn MemoryProvider>>,
    model_override: Option<&str>,
) -> Result<String> {
    let _timer = ScopedTimer::new("solve_task_repl");

    if input.enable_cache {
        if let Some(cached) = cache.get(task, &input.project) {
            info!(task = task, "cache hit");
            return Ok(cached);
        }
    }

    let executor = CodeExecutor::default_executor();
    let tools_block = format_tools_for_prompt(&input.custom_tools);
    let memory_block = build_memory_context(memory.as_ref(), task);
    let system_content = if tools_block.is_empty() && memory_block.is_empty() {
        REPL_SYSTEM.to_string()
    } else if memory_block.is_empty() {
        format!("{REPL_SYSTEM}\n\n{tools_block}")
    } else if tools_block.is_empty() {
        format!("{REPL_SYSTEM}\n\n{memory_block}")
    } else {
        format!("{REPL_SYSTEM}\n\n{tools_block}\n\n{memory_block}")
    };

    let model = model_override
        .or(input.model.as_deref())
        .unwrap_or("gpt-4o")
        .to_string();

    let sampling = crate::sampling::SamplingArgs::for_node_type(Action::Solve);

    let mut messages: Vec<Message> = vec![
        Message {
            role: Role::System,
            content: system_content,
        },
        Message {
            role: Role::User,
            content: format!("Task: {task}\n\nWrite code to solve this task. Use ```python or ```bash code blocks."),
        },
    ];

    let result =
        run_repl_loop(task, input, &llm, budget, &executor, sampling, &model, &mut messages)
            .await?;

    if input.enable_cache {
        cache.put(task, &input.project, &result);
    }

    Ok(result)
}

/// Run the REPL loop: call LLM, execute code, feed results back.
#[allow(clippy::too_many_arguments)]
async fn run_repl_loop(
    task: &str,
    input: &StartRunInput,
    llm: &Arc<dyn LlmBackend + Send + Sync>,
    budget: &RunBudget,
    executor: &CodeExecutor,
    sampling: crate::sampling::SamplingArgs,
    model: &str,
    messages: &mut Vec<Message>,
) -> Result<String> {
    for iteration in 0..MAX_REPL_ITERATIONS {
        let request = sampling.clone().apply_to_request(CompletionRequest {
            model: model.to_string(),
            messages: messages.clone(),
            temperature: None,
            max_tokens: Some(2048),
            stop: None,
        });

        let response = retry_with_backoff(&input.retry_policy.inner, || {
            let req = request.clone();
            let llm = llm.clone();
            async move { llm.complete(req).await }
        })
        .await?;

        budget.record_call(model, &response.usage);

        let blocks = find_code_blocks(&response.content);

        if blocks.is_empty() {
            info!(
                task = task,
                iterations = iteration + 1,
                tokens = response.usage.total_tokens,
                "repl task solved"
            );
            return Ok(response.content);
        }

        messages.push(Message {
            role: Role::Assistant,
            content: response.content.clone(),
        });

        let mut results_text = String::new();
        for block in &blocks {
            match executor.execute(block) {
                Ok(result) => {
                    results_text.push_str(&format_repl_result(&result));
                    results_text.push('\n');
                }
                Err(e) => {
                    results_text.push_str("[execution error: ");
                    results_text.push_str(&e);
                    results_text.push_str("]\n");
                }
            }
        }

        messages.push(Message {
            role: Role::User,
            content: format!("Code executed:\n{results_text}\nAnalyze the output and either write more code or provide your final answer."),
        });
    }

    // Exhausted iterations — ask for final answer
    messages.push(Message {
        role: Role::User,
        content: "You have reached the maximum number of code iterations. Provide your final answer now based on all the work done so far.".to_string(),
    });

    let final_request = sampling.apply_to_request(CompletionRequest {
        model: model.to_string(),
        messages: messages.clone(),
        temperature: None,
        max_tokens: Some(2048),
        stop: None,
    });

    let final_response = retry_with_backoff(&input.retry_policy.inner, || {
        let req = final_request.clone();
        let llm = llm.clone();
        async move { llm.complete(req).await }
    })
    .await?;

    budget.record_call(model, &final_response.usage);

    info!(
        task = task,
        iterations = MAX_REPL_ITERATIONS,
        "repl task solved (max iterations)"
    );

    Ok(final_response.content)
}

