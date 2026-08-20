pub mod local;
pub mod server;

use std::path::PathBuf;

use anyhow::Result;
use tokio::runtime::Runtime;

use crate::cli::{Cli, OutputFormatArg};
use crate::config::Config;
use crate::output::Format;

/// Resolve the effective output format from CLI flag and config file.
fn resolve_format(cli_format: Option<OutputFormatArg>, cfg_format: Option<&str>) -> Format {
    match cli_format {
        Some(OutputFormatArg::Json) => Format::Json,
        Some(OutputFormatArg::Tree) => Format::Tree,
        Some(OutputFormatArg::Markdown) => Format::Markdown,
        Some(OutputFormatArg::Prompt) => Format::Prompt,
        None => match cfg_format {
            Some("json") => Format::Json,
            Some("tree") => Format::Tree,
            Some("markdown") => Format::Markdown,
            Some("prompt") => Format::Prompt,
            _ => Format::Tree,
        },
    }
}

/// Entry point for command dispatch.
///
/// Resolves configuration precedence (CLI overrides config overrides defaults)
/// and routes to either the local command implementation or a remote gRPC
/// server when `--server` is supplied.
pub fn dispatch(cli: Cli, cfg: Config, rt: &Runtime) -> Result<()> {
    let project = cli
        .project
        .clone()
        .or_else(|| cfg.project.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    let backend = cli.backend.clone().or_else(|| cfg.backend.clone());
    let model = cli.model.clone().or_else(|| cfg.model.clone());
    let agent_name = cli.agent.clone().or_else(|| cfg.agent.name.clone());
    let format = resolve_format(cli.format, cfg.format.as_deref());

    if let Some(server_addr) = cli.server.clone() {
        return server::run_server(cli, server_addr, project, format, rt);
    }

    local::run_local(cli, cfg, project, backend, model, agent_name, format, rt)
}
