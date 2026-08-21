pub mod local;
pub mod server;

use std::path::PathBuf;

use anyhow::Result;
use tokio::runtime::Runtime;

use crate::cli::{Cli, Commands, OutputFormatArg};
use crate::client;
use crate::config::Config;
use crate::output::Format;

/// Resolve the effective output format from CLI flag and config file.
///
/// `default` is used when neither the CLI flag nor the config file specify a
/// format. Content-retrieval commands (search/context/query) pass
/// `Format::Jsonl` so an AI consumes only the matched file content; all other
/// commands keep `Format::Path`. `allow_jsonl` coerces an explicit `jsonl`
/// selection to `Format::FullJson` for non-content commands, which have no
/// simplified JSONL rendering.
fn resolve_format(
    cli_format: Option<OutputFormatArg>,
    cfg_format: Option<&str>,
    default: Format,
    allow_jsonl: bool,
) -> Format {
    let resolved = match cli_format {
        Some(OutputFormatArg::FullJson) => Format::FullJson,
        Some(OutputFormatArg::Path) => Format::Path,
        Some(OutputFormatArg::Markdown) => Format::Markdown,
        Some(OutputFormatArg::Text) => Format::Text,
        Some(OutputFormatArg::Jsonl) => Format::Jsonl,
        None => match cfg_format {
            Some("full_json") => Format::FullJson,
            Some("path") => Format::Path,
            Some("markdown") => Format::Markdown,
            Some("text") => Format::Text,
            Some("jsonl") => Format::Jsonl,
            _ => default,
        },
    };
    if !allow_jsonl && resolved == Format::Jsonl {
        Format::FullJson
    } else {
        resolved
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
    let is_content = matches!(
        cli.command,
        Commands::Search { .. } | Commands::Context { .. } | Commands::Query { .. }
    );
    let default = if is_content {
        Format::Text
    } else {
        Format::Path
    };
    let format = resolve_format(cli.format, cfg.format.as_deref(), default, is_content);

    // Remote gRPC server mode. Precedence: `--server` flag > config file /
    // env (`~/.arlm/config.toml` `[server] addr` or `ARLM_SERVER_ADDR`) >
    // local execution. When neither is set we fall through to local mode.
    let server_addr = cli.server.clone().or_else(client::explicit_addr);
    if let Some(addr) = server_addr {
        return server::run_server(cli, addr, project, format, rt);
    }

    local::run_local(cli, cfg, project, backend, model, agent_name, format, rt)
}
