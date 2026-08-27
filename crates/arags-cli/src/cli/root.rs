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

    /// Project path override (used by `init` / `persist`). This is the
    /// filesystem path of the project root, distinct from the canonical project
    /// *name* accepted by subcommand `--project` flags (e.g. `explore search`).
    #[arg(short = 'P', long = "project-path", global = true)]
    pub project_path: Option<PathBuf>,

    /// LLM backend name (overrides config).
    #[arg(long, global = true)]
    pub backend: Option<String>,

    /// Model name (overrides config).
    #[arg(long, global = true)]
    pub model: Option<String>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::cli::commands::ExploreCmd;
    use super::*;

    /// `arags explore search "q"` (no `--project`) must parse without panicking;
    /// the project scope then falls back to the configured/local project.
    #[test]
    fn explore_search_parses_without_project_flag() {
        let parsed = Cli::try_parse_from(["arags", "explore", "search", "q"]);
        assert!(
            parsed.is_ok(),
            "explore search without --project must parse: {:?}",
            parsed.err()
        );
    }

    /// `arags explore search "q" --project sucesu` previously panicked with a
    /// clap arg-id collision between the global `--project` (path) flag and the
    /// subcommand `--project` (name) flag. It must now parse and carry the value.
    #[test]
    fn explore_search_parses_with_project_flag() {
        let parsed =
            Cli::try_parse_from(["arags", "explore", "search", "q", "--project", "sucesu"]);
        assert!(
            parsed.is_ok(),
            "explore search --project must parse without panicking: {:?}",
            parsed.err()
        );
        match parsed.unwrap().command {
            Commands::Explore {
                cmd: ExploreCmd::Search { project, query, .. },
            } => {
                assert_eq!(query, "q");
                assert_eq!(project.as_deref(), Some("sucesu"));
            }
            other => panic!("expected ExploreCmd::Search, got {other:?}"),
        }
    }
}
