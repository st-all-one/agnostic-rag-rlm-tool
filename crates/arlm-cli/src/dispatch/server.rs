use std::path::PathBuf;

use anyhow::{Result, bail};
use tonic::Request;
use tracing::{debug, instrument, warn};

use tokio::runtime::Runtime;

use crate::cli::{Cli, Commands, SessionAction};
use crate::client;
use crate::commands::persist;
use crate::output::Format;

/// Map a textual tier (`fts`/`entity`/`vector`/`auto`) onto the proto enum.
fn map_search_tier(tier: &str) -> arlm_proto::proto::SearchTier {
    debug!(tier, "resolving search tier");
    match tier {
        "fts" => arlm_proto::proto::SearchTier::TierBm25,
        "entity" => arlm_proto::proto::SearchTier::TierEntity,
        "vector" => arlm_proto::proto::SearchTier::TierSemantic,
        _ => arlm_proto::proto::SearchTier::TierHybrid,
    }
}

/// Dispatch commands to a remote `arlm-server` over gRPC.
#[instrument(skip(rt), fields(server = %server_addr))]
pub fn run_server(
    cli: Cli,
    server_addr: String,
    project: PathBuf,
    format: Format,
    rt: &Runtime,
) -> Result<()> {
    let client_config = client::ClientConfig { addr: server_addr };
    let mut grpc_client = rt.block_on(client::create_client(&client_config))?;
    let project_str = project.to_string_lossy().to_string();

    match cli.command {
        Commands::Run {
            task,
            backend: cmd_backend,
            model: cmd_model,
            depth,
            max_nodes,
            persist: cmd_persist,
            ..
        } => {
            let request = Request::new(arlm_proto::proto::RunRequest {
                project: project_str,
                task: task.clone(),
                backend: cmd_backend.unwrap_or_default(),
                model: cmd_model.unwrap_or_default(),
                options: Some(arlm_proto::proto::RunOptions {
                    max_depth: depth as i32,
                    max_iterations: max_nodes as i32,
                    ..Default::default()
                }),
            });
            let response = rt.block_on(grpc_client.start_run(request))?;
            let run_id = response.into_inner().run_id;

            let rendered = match format {
                Format::Json => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({ "run_id": run_id }))
                    .to_json_string(),
                _ => format!("Run started: {run_id}"),
            };
            print!("{rendered}");

            if cmd_persist {
                if let Err(e) = persist::save_page(&task, &rendered, &project, format) {
                    warn!(error = %e, "failed to persist run output");
                }
            }
            Ok(())
        }
        Commands::Search {
            query,
            top_k,
            tier,
            persist: cmd_persist,
            ..
        } => {
            let request = Request::new(arlm_proto::proto::SearchRequest {
                project: project_str,
                query: query.clone(),
                max_results: top_k as i32,
                tier: map_search_tier(&tier) as i32,
                ..Default::default()
            });
            let response = rt.block_on(grpc_client.search(request))?;
            let results = response.into_inner().results;

            let rendered = match format {
                Format::Json => {
                    let items: Vec<serde_json::Value> = results
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "chunk_id": r.chunk_id,
                                "file": r.file_path,
                                "score": r.score,
                                "text": r.text,
                            })
                        })
                        .collect();
                    crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({
                            "query": query,
                            "results": items,
                            "count": results.len(),
                        }))
                        .to_json_string()
                }
                Format::Tree => {
                    let items: Vec<crate::output::tree::SearchResultItem> = results
                        .iter()
                        .map(|r| crate::output::tree::SearchResultItem {
                            file_path: r.file_path.clone(),
                            line_start: i64::from(r.start_line),
                            line_end: i64::from(r.end_line),
                            score: r.score,
                        })
                        .collect();
                    crate::output::tree::render_search_results(&items)
                }
                Format::Markdown => {
                    let items: Vec<crate::output::markdown::SuperItem> = results
                        .iter()
                        .map(|r| crate::output::markdown::SuperItem {
                            file_path: r.file_path.clone(),
                            score: r.score,
                            content: r.text.clone(),
                            language: None,
                        })
                        .collect();
                    crate::output::markdown::render_search_results(&items)
                }
                Format::Prompt => {
                    let items: Vec<crate::output::prompt::PromptItem> = results
                        .iter()
                        .map(|r| crate::output::prompt::PromptItem {
                            file_path: r.file_path.clone(),
                            score: r.score,
                            content: r.text.clone(),
                            language: None,
                        })
                        .collect();
                    crate::output::prompt::render_search_context(&items)
                }
            };
            print!("{rendered}");

            if cmd_persist {
                if let Err(e) = persist::save_page(&query, &rendered, &project, format) {
                    warn!(error = %e, "failed to persist search output");
                }
            }
            Ok(())
        }
        Commands::Context {
            task,
            persist: cmd_persist,
            ..
        } => {
            let request = Request::new(arlm_proto::proto::ContextRequest {
                project: project_str,
                task: task.clone(),
                ..Default::default()
            });
            let response = rt.block_on(grpc_client.build_context(request))?;
            let ctx = response.into_inner().context;

            let rendered = match format {
                Format::Json => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({ "task": task, "context": ctx }))
                    .to_json_string(),
                _ => ctx,
            };
            print!("{rendered}");

            if cmd_persist {
                if let Err(e) = persist::save_page(&task, &rendered, &project, format) {
                    warn!(error = %e, "failed to persist context output");
                }
            }
            Ok(())
        }
        Commands::Status { run_id } => {
            let rendered = if let Some(rid) = run_id {
                let request = Request::new(rid.clone());
                let response = rt.block_on(grpc_client.get_run(request))?;
                let run = response.into_inner();
                match format {
                    Format::Json => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({
                            "run_id": run.run_id,
                            "status": run.status,
                        }))
                        .to_json_string(),
                    _ => format!("Run {}: {}", run.run_id, run.status),
                }
            } else {
                let request = Request::new(());
                let response = rt.block_on(grpc_client.get_server_status(request))?;
                let status = response.into_inner();
                match format {
                    Format::Json => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({
                            "version": status.version,
                            "total_projects": status.total_projects,
                            "total_chunks": status.total_chunks,
                        }))
                        .to_json_string(),
                    _ => format!(
                        "Server v{} - {} projects, {} chunks",
                        status.version, status.total_projects, status.total_chunks
                    ),
                }
            };
            print!("{rendered}");
            Ok(())
        }
        Commands::Session { action } => match action {
            SessionAction::Create { title } => {
                let request = Request::new(arlm_proto::proto::CreateSessionRequest {
                    project: project_str,
                    title: title.clone(),
                });
                let response = rt.block_on(grpc_client.create_session(request))?;
                let sid = response.into_inner().session_id;
                let rendered = match format {
                    Format::Json => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({ "session_id": sid }))
                        .to_json_string(),
                    _ => format!("Session created: {sid}"),
                };
                print!("{rendered}");
                Ok(())
            }
            SessionAction::Resume { session_id } => {
                let request = Request::new(session_id.clone());
                let response = rt.block_on(grpc_client.get_session(request))?;
                let session = response.into_inner();
                let rendered = match format {
                    Format::Json => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({
                            "session_id": session.session_id,
                            "turn_count": session.turn_count,
                        }))
                        .to_json_string(),
                    _ => format!(
                        "Session: {} - {} turns",
                        session.session_id, session.turn_count
                    ),
                };
                print!("{rendered}");
                Ok(())
            }
            SessionAction::List => {
                let request = Request::new(project_str);
                let response = rt.block_on(grpc_client.list_sessions(request))?;
                let sessions = response.into_inner().sessions;
                let rendered = match format {
                    Format::Json => {
                        let items: Vec<serde_json::Value> = sessions
                            .iter()
                            .map(|s| {
                                serde_json::json!({
                                    "session_id": s.session_id,
                                    "title": s.title,
                                    "turn_count": s.turn_count,
                                })
                            })
                            .collect();
                        crate::output::json::JsonOutput::ok()
                            .with_data(serde_json::json!({ "sessions": items }))
                            .to_json_string()
                    }
                    _ => sessions
                        .iter()
                        .map(|s| format!("{}: {}", s.session_id, s.title))
                        .collect::<Vec<_>>()
                        .join("\n"),
                };
                print!("{rendered}");
                Ok(())
            }
        },
        Commands::Cost { run_id, .. } => {
            if let Some(rid) = run_id {
                let request = Request::new(rid.clone());
                let response = rt.block_on(grpc_client.get_run(request))?;
                let run = response.into_inner();
                let rendered = match format {
                    Format::Json => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({
                            "run_id": run.run_id,
                            "total_cost": run.total_cost,
                        }))
                        .to_json_string(),
                    _ => format!("Run {} cost: ${:.6}", run.run_id, run.total_cost),
                };
                print!("{rendered}");
            }
            Ok(())
        }
        Commands::Index { path, .. } => {
            let request = Request::new(arlm_proto::proto::IndexRequest {
                project: project_str.clone(),
                root_path: path.to_string_lossy().to_string(),
                ..Default::default()
            });
            let response = rt.block_on(grpc_client.index_project(request))?;
            let resp = response.into_inner();
            let rendered = match format {
                Format::Json => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({
                        "files_indexed": resp.files_indexed,
                        "chunks_created": resp.chunks_created,
                        "duration_ms": resp.duration_ms,
                    }))
                    .to_json_string(),
                _ => format!(
                    "Indexed {} files, {} chunks ({:.1} ms)",
                    resp.files_indexed, resp.chunks_created, resp.duration_ms
                ),
            };
            print!("{rendered}");
            Ok(())
        }
        Commands::Query { question, .. } => {
            let request = Request::new(arlm_proto::proto::ContextRequest {
                project: project_str.clone(),
                task: question.clone(),
                ..Default::default()
            });
            let response = rt.block_on(grpc_client.build_context(request))?;
            let ctx = response.into_inner().context;
            let rendered = match format {
                Format::Json => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({ "question": question, "context": ctx }))
                    .to_json_string(),
                _ => ctx,
            };
            print!("{rendered}");
            Ok(())
        }
        _ => {
            bail!(
                "Server mode does not support this command. Supported commands in server mode: \
                 index, search, status, session, run, cost, context, query. Other commands \
                 (history, consolidate, decay, cancel, checkpoints, restore-page, wiki, entities, \
                 persist, serve) must be run locally."
            );
        }
    }
}
