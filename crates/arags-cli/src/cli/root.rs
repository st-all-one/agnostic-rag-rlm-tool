use std::path::PathBuf;

use clap::Parser;

use super::commands::Commands;

/// Output format selected on the command line.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormatArg {
    #[value(name = "full_json")]
    FullJson,
    #[value(name = "path")]
    Path,
    #[value(name = "markdown")]
    Markdown,
    /// `text` is the agent-facing prompt context format (formerly `prompt`).
    #[value(name = "text")]
    Text,
    #[value(name = "jsonl")]
    Jsonl,
}

/// arags command-line interface.
#[derive(Parser, Debug)]
#[command(
    name = "arags",
    about = "Agnostic RAG Server — on-demand, agent-agnostic CLI (pure gRPC client)",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output with structured logging.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Output format: full_json, path, markdown, text, jsonl.
    ///
    /// `path` prints the relative file path (human tree for search). `text`
    /// renders the agent-facing prompt context. `jsonl` (default for
    /// search/query) emits a single `{"query":..,"results":[{"file","text"}]}`
    /// object so an AI can consume only the needed content.
    #[arg(short, long, global = true)]
    pub format: Option<OutputFormatArg>,

    /// Project path.
    #[arg(short, long, global = true)]
    pub project: Option<PathBuf>,

    /// LLM backend name (overrides config).
    #[arg(long, global = true)]
    pub backend: Option<String>,

    /// Model name (overrides config).
    #[arg(long, global = true)]
    pub model: Option<String>,
}
