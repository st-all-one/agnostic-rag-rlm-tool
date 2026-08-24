use std::path::PathBuf;

use anyhow::Result;
use tracing::instrument;

use tokio::runtime::Runtime;

use crate::cli::{Cli, Commands, SessionAction, parse_tool_arg};
use crate::commands;
use crate::config::Config;
use crate::output::Format;

/// Dispatch commands to the local implementation.
#[instrument(skip(rt, cfg))]
pub fn run_local(
    cli: Cli,
    cfg: Config,
    project: PathBuf,
    backend: Option<String>,
    model: Option<String>,
    agent_name: Option<String>,
    format: Format,
    rt: &Runtime,
) -> Result<()> {
    match cli.command {
        Commands::Run {
            task,
            llm,
            backend: cmd_backend,
            model: cmd_model,
            depth,
            max_nodes,
            concurrency,
            max_budget,
            live,
            agent: cmd_agent,
            tools: cmd_tools,
            session: cmd_session,
            repl: cmd_repl,
            persist: cmd_persist,
        } => {
            let custom_tools: Vec<arlm_core::CustomTool> =
                cmd_tools.iter().filter_map(|t| parse_tool_arg(t)).collect();
            rt.block_on(commands::run::execute(commands::run::RunConfig {
                task: &task,
                llm,
                backend: cmd_backend.as_deref().or(backend.as_deref()),
                model: cmd_model.as_deref().or(model.as_deref()),
                depth,
                max_nodes,
                concurrency,
                max_budget,
                project: &project,
                format,
                verbose: cli.verbose,
                live,
                agent: cmd_agent.as_deref().or(agent_name.as_deref()),
                custom_tools,
                session_id: cmd_session.as_deref(),
                repl: cmd_repl,
                persist: cmd_persist,
            }))
        }
        Commands::Index {
            path,
            chunk_size,
            ignore_patterns,
            force_include,
            watch,
        } => rt.block_on(commands::index::execute(commands::index::IndexConfig {
            path: &path,
            chunk_size,
            ignore_patterns: &ignore_patterns,
            force_include: &force_include,
            watch,
            project: &project,
            format,
            verbose: cli.verbose,
            config: &cfg,
        })),
        Commands::Search {
            query,
            top_k,
            file_pattern,
            min_score,
            all,
            tier,
            max_tokens,
            persist: cmd_persist,
        } => rt.block_on(commands::search::execute(commands::search::SearchConfig {
            query: &query,
            top_k: cfg.search.top_k.unwrap_or(top_k as u32) as usize,
            file_pattern: file_pattern.as_deref(),
            min_score,
            all,
            tier: &tier,
            max_tokens: if max_tokens == 0 {
                cfg.search.max_tokens
            } else {
                Some(max_tokens)
            },
            project: &project,
            format,
            verbose: cli.verbose,
            persist: cmd_persist,
            config: &cfg,
        })),
        Commands::Query {
            question,
            backend: cmd_backend,
            model: cmd_model,
            llm,
            ..
        } => rt.block_on(commands::query::execute(commands::query::QueryConfig {
            question: &question,
            backend: cmd_backend.as_deref().or(backend.as_deref()),
            model: cmd_model.as_deref().or(model.as_deref()),
            project: &project,
            format,
            verbose: cli.verbose,
            llm,
            config: &cfg,
        })),
        Commands::Context {
            task,
            top_k,
            all,
            tier,
            max_tokens,
            persist: cmd_persist,
        } => rt.block_on(commands::context::execute(
            commands::context::ContextConfig {
                task: &task,
                top_k: cfg.search.top_k.unwrap_or(top_k as u32) as usize,
                all,
                tier: &tier,
                max_tokens: if max_tokens == 0 {
                    cfg.search.max_tokens
                } else {
                    Some(max_tokens)
                },
                project: &project,
                format,
                verbose: cli.verbose,
                persist: cmd_persist,
            },
        )),
        Commands::Status { run_id } => {
            commands::status::execute(run_id.as_deref(), &project, format)
        }
        Commands::History { limit } => {
            commands::history::execute(commands::history::HistoryConfig {
                limit,
                project: &project,
                format,
            })
        }
        Commands::Cost {
            run_id,
            agent: cmd_agent,
        } => commands::cost::execute(
            run_id.as_deref(),
            cmd_agent.as_deref().or(agent_name.as_deref()),
            &project,
            format,
        ),
        Commands::Session { action } => match action {
            SessionAction::Create { title } => {
                commands::session::execute_create(&title, &project, format)
            }
            SessionAction::Resume { session_id } => {
                commands::session::execute_resume(&session_id, &project, format)
            }
            SessionAction::List => commands::session::execute_list(&project, format),
        },
        Commands::Consolidate => {
            commands::consolidate::execute(commands::consolidate::ConsolidateConfig {
                project: &project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Decay { dry_run } => commands::decay::execute(commands::decay::DecayArgs {
            dry_run,
            project: &project,
            format,
        }),
        Commands::Cancel { run_id } => commands::cancel::execute(&run_id, &project, format),
        Commands::Checkpoints { run_id } => {
            commands::checkpoints::execute(run_id.as_deref(), format)
        }
        Commands::RestorePage { page_name } => {
            commands::restore_page::execute(&page_name, &project, format)
        }
        Commands::Wiki { action } => commands::wiki::execute(&action, &project, format),
        Commands::Entities { query } => commands::entities::execute(&query, &project, format),
        Commands::Persist { title, query } => {
            commands::persist::execute(commands::persist::PersistArgs {
                title,
                query,
                project: &project,
                format,
            })
        }
        Commands::Serve { port, host, mcp } => {
            rt.block_on(commands::serve::execute(commands::serve::ServeConfig {
                port,
                host: &host,
                project: &project,
                verbose: cli.verbose,
                mcp,
            }))
        }
        Commands::Cache { .. } => Err(anyhow::anyhow!(
            "`arlm cache` requires a running arlm-server (use --server)",
        )),
    }
}
