use std::path::Path;

use crate::output::Format;

/// Configuration for `arlm run`.
///
/// Mirrors the CLI surface for the `run` subcommand. Field names and types
/// are part of the stable API consumed by `crate::dispatch::local`.
#[allow(clippy::struct_excessive_bools)]
pub struct RunConfig<'a> {
    pub task: &'a str,
    pub llm: bool,
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub depth: u32,
    pub max_nodes: u32,
    pub concurrency: usize,
    pub max_budget: f64,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
    pub live: bool,
    pub agent: Option<&'a str>,
    pub custom_tools: Vec<arlm_core::CustomTool>,
    pub session_id: Option<&'a str>,
    pub repl: bool,
    pub persist: bool,
}
