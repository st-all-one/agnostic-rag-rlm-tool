//! Server command dispatch, split by responsibility:
//!
//! - [`index`]: upload/streaming of files (`arags index`)
//! - [`discover`]: file discovery + ignore rule composition
//! - [`projects`]: watcher registration (`--register/--unregister`)
//! - [`watch_daemon`]: hidden `__watch` daemon (quiet-window re-index)
//! - [`search`]: `search`/`query` RPCs and rendering
//! - [`memory_history`]: memory/cache admin + history RPCs
//! - [`init`]: `arags init` scaffolding of `.arags.toml`

pub mod discover;
pub mod exploration;
pub mod index;
pub mod init;
pub mod memory_history;
pub mod projects;
pub mod search;
pub mod watch_daemon;

use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::runtime::Runtime;
use tracing::debug;

use crate::auth_client::AragsClient;
use crate::cli::{Cli, Commands, OutputFormatArg};
use crate::client::ClientConfig;
use crate::commands::persist::run_persist;
use crate::output::Format;
use crate::user_config::{self, EffectiveUserConfig};

/// Connect to the server, performing `AuthRefresh` when a refresh token is
/// configured, and returning a client that auto-attaches the session token.
pub(crate) fn connect(rt: &Runtime, cfg: &EffectiveUserConfig) -> Result<AragsClient> {
    let client_config = ClientConfig {
        addr: cfg.server_addr(),
        tls_ca: cfg.server.tls_ca.clone(),
        tls_cert: cfg.server.tls_cert.clone(),
        tls_key: cfg.server.tls_key.clone(),
    };
    let auth = cfg.auth().cloned().unwrap_or_default();
    let (client, _token) = crate::auth_client::connect(rt, &client_config, &auth)?;
    Ok(client)
}

/// Map a textual tier (`fts`/`entity`/`vector`/`hybrid`/`summary`/`auto`)
/// onto the proto enum. `auto` (and anything unknown) sends `UNSPECIFIED` so
/// the server applies its `[search].tier` default (plan 020).
fn map_search_tier(tier: &str) -> arags_proto::proto::SearchTier {
    use arags_proto::proto::SearchTier;
    debug!(tier, "resolving search tier");
    match tier {
        "fts" | "bm25" => SearchTier::TierBm25,
        "entity" => SearchTier::TierEntity,
        "vector" | "semantic" => SearchTier::TierSemantic,
        "hybrid" => SearchTier::TierHybrid,
        "summary" | "summaries" | "rlm" => SearchTier::TierSummary,
        _ => SearchTier::Unspecified,
    }
}

/// Entry point for command dispatch.
///
/// The CLI is a **pure gRPC client** (plus the user's local LLM for digest /
/// summarize). Every data command is routed to a remote `arags-server` over
/// gRPC; there is no local data plane (plan 020, D3).
pub fn dispatch(cli: Cli, rt: &Runtime) -> Result<()> {
    let cfg = user_config::load().unwrap_or_default();

    let project = cli
        .project_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("."));

    let is_content = matches!(
        cli.command,
        Commands::Search { .. } | Commands::Query { .. } | Commands::Ask { .. }
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

    run(cli, cfg, project, format, rt)
}

/// Route the parsed command to its module.
#[tracing::instrument(skip(cfg, rt))]
fn run(
    cli: Cli,
    cfg: EffectiveUserConfig,
    project: PathBuf,
    format: Format,
    rt: &Runtime,
) -> Result<()> {
    match cli.command {
        Commands::Init {
            name,
            ignore,
            server_addr,
            register,
            no_register,
            no_index,
            non_interactive,
            ..
        } => {
            let flags = init::InitFlags {
                name,
                ignore,
                server_addr,
                register: register && !no_register,
                do_index: !no_index,
                non_interactive,
            };
            init::run_init(rt, &cfg, &project, &flags, format)
        }
        Commands::Volunteer { once } => crate::volunteer::run(rt, &cfg, once),
        Commands::Index {
            path,
            ignore_patterns,
            force_include,
            register,
            unregister,
        } => {
            if unregister {
                return projects::run_unregister(&path);
            }
            let canonical = user_config::resolve_canonical_name(&cfg)?;
            let absolute = std::fs::canonicalize(&path)
                .with_context(|| format!("failed to resolve path: {}", path.display()))?;
            let mut client = connect(rt, &cfg)?;
            let result = index::run_index(
                rt,
                &mut client,
                &path,
                &canonical,
                &ignore_patterns,
                &force_include,
                format,
            );
            if result.is_ok() && register {
                projects::run_register(&absolute, &canonical)?;
            }
            result
        }
        Commands::WatchDaemon { root } => {
            let absolute = std::fs::canonicalize(&root)
                .with_context(|| format!("failed to resolve path: {}", root.display()))?;
            watch_daemon::run_watch_daemon(rt, &cfg, &absolute)
        }
        Commands::Search {
            query,
            top_k,
            tier,
            min_score,
            file_pattern,
            context,
            as_of_epoch,
            as_of,
            ..
        } => {
            let canonical = user_config::resolve_canonical_name(&cfg)?;
            let mut client = connect(rt, &cfg)?;
            let epoch = crate::cli::commands::resolve_as_of_epoch(as_of_epoch, as_of)?;
            if context {
                search::run_search_context(rt, &mut client, &canonical, &query, epoch, format)
            } else {
                search::run_search(
                    rt,
                    &mut client,
                    &canonical,
                    &query,
                    top_k,
                    &tier,
                    min_score,
                    file_pattern.as_deref(),
                    epoch,
                    format,
                )
            }
        }
        Commands::Query {
            question,
            cache_id,
            qa,
            backend,
            model,
            as_of_epoch,
            as_of,
        } => {
            let canonical = user_config::resolve_canonical_name(&cfg)?;
            let mut client = connect(rt, &cfg)?;
            let epoch = crate::cli::commands::resolve_as_of_epoch(as_of_epoch, as_of)?;
            search::run_query_deprecated(
                rt,
                &mut client,
                &canonical,
                &question,
                cache_id,
                qa,
                backend.as_deref(),
                model.as_deref(),
                epoch,
                format,
            )
        }
        Commands::Ask {
            question,
            cache_id,
            backend,
            model,
            as_of_epoch,
            as_of,
        } => {
            let canonical = user_config::resolve_canonical_name(&cfg)?;
            let mut client = connect(rt, &cfg)?;
            let epoch = crate::cli::commands::resolve_as_of_epoch(as_of_epoch, as_of)?;
            search::run_ask(
                rt,
                &mut client,
                &canonical,
                &question,
                cache_id,
                backend.as_deref(),
                model.as_deref(),
                epoch,
                format,
            )
        }
        Commands::Explore { cmd } => {
            let canonical = user_config::resolve_canonical_name(&cfg)?;
            let mut client = connect(rt, &cfg)?;
            exploration::run_explore(rt, &mut client, &canonical, cmd, format)
        }
        Commands::Maintenance { cmd } => {
            let mut client = connect(rt, &cfg)?;
            memory_history::run_memory(rt, &mut client, cmd, &project, format)
        }
        Commands::Persist { response_id, title } => {
            let mut client = connect(rt, &cfg)?;
            run_persist(
                rt,
                &mut client,
                &cfg,
                &project,
                &response_id,
                title.as_deref(),
                format,
            )
        }
        Commands::History { limit, user } => {
            let mut client = connect(rt, &cfg)?;
            memory_history::run_history(rt, &mut client, &project, limit, user.as_deref(), format)
        }
    }
}
