use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tonic::Request;
use tracing::{debug, instrument, warn};

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use arlm_proto::proto::index_chunk;
use arlm_proto::proto::{IndexChunk, IndexFile, IndexInit, IndexResponse};

use crate::auth_client::ArlmClient;
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
    let auth_cfg = crate::config::Config::load()
        .map(|c| c.auth)
        .unwrap_or_default();
    let mut grpc_client = crate::auth_client::connect(rt, &client_config, &auth_cfg)?;
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
                Format::FullJson => crate::output::json::JsonOutput::ok()
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
            min_score,
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
            let mut results = response.into_inner().results;
            // Server scores are normalised to [0, 1] (higher = better), so a
            // minimum-score threshold keeps the strongest matches.
            if let Some(min) = min_score {
                results.retain(|r| r.score >= min);
            }

            let rendered = match format {
                Format::FullJson => {
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
                Format::Jsonl => {
                    let pairs: Vec<(String, String)> = results
                        .iter()
                        .map(|r| (r.file_path.clone(), r.text.clone()))
                        .collect();
                    crate::output::jsonl::render_content_jsonl("query", &query, &pairs)
                }
                Format::Path => {
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
                Format::Text => {
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
                project: project_str.clone(),
                task: task.clone(),
                ..Default::default()
            });
            let response = rt.block_on(grpc_client.build_context(request))?;
            let ctx = response.into_inner().context;

            let rendered = match format {
                Format::FullJson => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({ "task": task, "context": ctx }))
                    .to_json_string(),
                Format::Jsonl => {
                    let request = Request::new(arlm_proto::proto::SearchRequest {
                        project: project_str.clone(),
                        query: task.clone(),
                        max_results: 10,
                        tier: arlm_proto::proto::SearchTier::TierHybrid as i32,
                        ..Default::default()
                    });
                    let results = rt
                        .block_on(grpc_client.search(request))?
                        .into_inner()
                        .results;
                    let pairs: Vec<(String, String)> = results
                        .iter()
                        .map(|r| (r.file_path.clone(), r.text.clone()))
                        .collect();
                    crate::output::jsonl::render_content_jsonl("task", &task, &pairs)
                }
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
                    Format::FullJson => crate::output::json::JsonOutput::ok()
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
                    Format::FullJson => crate::output::json::JsonOutput::ok()
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
                    Format::FullJson => crate::output::json::JsonOutput::ok()
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
                    Format::FullJson => crate::output::json::JsonOutput::ok()
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
                    Format::FullJson => {
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
                    Format::FullJson => crate::output::json::JsonOutput::ok()
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
        Commands::Index {
            path,
            ignore_patterns,
            force_include,
            ..
        } => {
            let absolute = std::fs::canonicalize(&path)
                .with_context(|| format!("failed to resolve path: {}", path.display()))?;
            let project_str = project.to_string_lossy().to_string();

            // Discover files locally — the client owns the filesystem, never the
            // server. Sensitive/ignored paths are skipped unless force-included.
            let files = arlm_embedding::pipeline::discover_files(
                &absolute,
                &ignore_patterns,
                &force_include,
            )
            .map_err(|e| anyhow::anyhow!("file discovery failed: {e}"))?;

            if files.is_empty() {
                let rendered = match format {
                    Format::FullJson => crate::output::json::JsonOutput::ok()
                        .with_data(serde_json::json!({ "files_indexed": 0, "chunks_created": 0 }))
                        .to_json_string(),
                    _ => format!("No files to index in {}", absolute.display()),
                };
                print!("{rendered}");
                return Ok(());
            }

            // Fan out across several concurrent gRPC streams for throughput.
            let parallelism = std::thread::available_parallelism()
                .map_or(4, std::num::NonZero::get)
                .clamp(1, 8);
            let groups = partition_files(&files, parallelism);
            let total = files.len() as u64;

            let progress = Arc::new(indicatif::ProgressBar::new(total));
            progress.set_style(
                indicatif::ProgressStyle::default_bar()
                    .template("{spinner:.green} [{bar:30.cyan/blue}] {pos}/{len} files ({eta})")
                    .map_err(|e| anyhow::anyhow!("invalid progress template: {e}"))?,
            );
            progress.set_message("Uploading");

            let mut totals = (0i64, 0i64);
            let mut handles = Vec::with_capacity(groups.len());
            for group in groups {
                let mut client = grpc_client.clone();
                let pb = progress.clone();
                let project = project_str.clone();
                let root = absolute.clone();
                let handle = rt.spawn(async move {
                    stream_index_group(&mut client, project, root, group, pb).await
                });
                handles.push(handle);
            }
            for handle in handles {
                let (files_idx, chunks_idx) = rt
                    .block_on(handle)
                    .map_err(|e| anyhow::anyhow!("upload task failed: {e}"))??;
                totals.0 += files_idx;
                totals.1 += chunks_idx;
            }
            progress.finish_and_clear();

            let rendered = match format {
                Format::FullJson => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({
                        "files_indexed": totals.0,
                        "chunks_created": totals.1,
                    }))
                    .to_json_string(),
                _ => format!("Indexed {} files, {} chunks", totals.0, totals.1),
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
                Format::FullJson => crate::output::json::JsonOutput::ok()
                    .with_data(serde_json::json!({ "question": question, "context": ctx }))
                    .to_json_string(),
                Format::Jsonl => {
                    let request = Request::new(arlm_proto::proto::SearchRequest {
                        project: project_str.clone(),
                        query: question.clone(),
                        max_results: 10,
                        tier: arlm_proto::proto::SearchTier::TierHybrid as i32,
                        ..Default::default()
                    });
                    let results = rt
                        .block_on(grpc_client.search(request))?
                        .into_inner()
                        .results;
                    let pairs: Vec<(String, String)> = results
                        .iter()
                        .map(|r| (r.file_path.clone(), r.text.clone()))
                        .collect();
                    crate::output::jsonl::render_content_jsonl("question", &question, &pairs)
                }
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

/// Stream one disjoint group of files to the server as a single `IndexProject`
/// client-stream, returning the files/chunks counts reported by the server.
async fn stream_index_group(
    client: &mut ArlmClient,
    project: String,
    root: PathBuf,
    files: Vec<PathBuf>,
    progress: Arc<indicatif::ProgressBar>,
) -> anyhow::Result<(i64, i64)> {
    let (tx, rx) = mpsc::channel::<IndexChunk>(32);
    let stream = ReceiverStream::new(rx);
    let response_fut = client.index_project(stream);

    let send_handle = tokio::spawn(async move {
        if tx
            .send(IndexChunk {
                body: Some(index_chunk::Body::Init(IndexInit {
                    project,
                    root_path: root.to_string_lossy().to_string(),
                    force_include: vec![],
                    exclude_patterns: vec![],
                })),
            })
            .await
            .is_err()
        {
            return;
        }

        for file in &files {
            let Ok(content) = std::fs::read_to_string(file) else {
                progress.inc(1);
                continue;
            };
            let rel_path = file
                .strip_prefix(&root)
                .unwrap_or(file)
                .to_string_lossy()
                .to_string();
            let compressed = arlm_embedding::pipeline::compress_text(&content);
            if tx
                .send(IndexChunk {
                    body: Some(index_chunk::Body::File(IndexFile {
                        rel_path,
                        content: compressed,
                        compressed: true,
                        size_bytes: i64::try_from(content.len()).unwrap_or(i64::MAX),
                    })),
                })
                .await
                .is_err()
            {
                break;
            }
            progress.inc(1);
        }
    });

    let response = response_fut
        .await
        .map_err(|e| anyhow::anyhow!("index stream failed: {e}"))?;
    send_handle
        .await
        .map_err(|e| anyhow::anyhow!("upload task failed: {e}"))?;

    let inner: IndexResponse = response.into_inner();
    Ok((inner.files_indexed, inner.chunks_created))
}

/// Split `files` into `n` roughly equal, disjoint groups for parallel upload.
#[must_use]
fn partition_files(files: &[PathBuf], n: usize) -> Vec<Vec<PathBuf>> {
    let n = n.max(1).min(files.len().max(1));
    let mut groups: Vec<Vec<PathBuf>> = (0..n).map(|_| Vec::new()).collect();
    for (i, file) in files.iter().enumerate() {
        groups[i % n].push(file.clone());
    }
    groups.retain(|g| !g.is_empty());
    groups
}
