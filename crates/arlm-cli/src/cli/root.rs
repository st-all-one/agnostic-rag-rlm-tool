use std::path::PathBuf;

use clap::Parser;

use super::commands::Commands;

/// Output format selected on the command line.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormatArg {
    Json,
    Tree,
    Markdown,
    Prompt,
}

/// arlm command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "arlm",
    about = "Agnostic RLM — Recursive Language Model CLI",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output with structured logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Output format: json, tree, markdown, prompt
    #[arg(short, long, global = true)]
    pub format: Option<OutputFormatArg>,

    /// Project path
    #[arg(short, long, global = true)]
    pub project: Option<PathBuf>,

    /// Config file path (default: ~/.arlm/config.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// LLM backend (overrides config)
    #[arg(long, global = true)]
    pub backend: Option<String>,

    /// Model name (overrides config)
    #[arg(long, global = true)]
    pub model: Option<String>,

    /// Agent name (overrides config)
    #[arg(long, global = true)]
    pub agent: Option<String>,

    /// Connect to a running arlm-server instead of running locally
    #[arg(long, global = true)]
    pub server: Option<String>,
}

/// Parse a `--tool` argument in format "name:description" or
/// "name:param1,param2:description".
#[must_use]
pub fn parse_tool_arg(arg: &str) -> Option<arlm_core::CustomTool> {
    let (name_part, description) = arg.split_once(':')?;
    let name = name_part.trim().to_string();
    let description = description.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(arlm_core::CustomTool::function(&name, &description))
}
