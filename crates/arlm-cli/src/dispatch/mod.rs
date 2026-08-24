pub mod server;

use std::path::PathBuf;

use anyhow::Result;
use tokio::runtime::Runtime;

use crate::cli::{Cli, Commands, OutputFormatArg};
use crate::output::Format;
use crate::user_config;

/// Entry point for command dispatch.
///
/// The CLI is a **pure gRPC client** (plus the user's local LLM for digest /
/// summarize). Every data command is routed to a remote `arlm-server` over
/// gRPC; there is no local data plane (plan 020, D3).
pub fn dispatch(cli: Cli, rt: &Runtime) -> Result<()> {
    let cfg = user_config::load().unwrap_or_default();

    let project = cli.project.clone().unwrap_or_else(|| PathBuf::from("."));

    let is_content = matches!(
        cli.command,
        Commands::Search { .. } | Commands::Query { .. }
    );
    let default = if is_content {
        Format::Text
    } else {
        Format::Path
    };
    let format = match cli.format {
        Some(OutputFormatArg::FullJson) => Format::FullJson,
        Some(OutputFormatArg::Path) => Format::Path,
        Some(OutputFormatArg::Markdown) => Format::Markdown,
        Some(OutputFormatArg::Text) => Format::Text,
        Some(OutputFormatArg::Jsonl) => Format::Jsonl,
        None => default,
    };

    server::run(cli, cfg, project, format, rt)
}
