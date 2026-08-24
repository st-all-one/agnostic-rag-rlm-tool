use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tonic::Request;
use tracing::{debug, instrument};

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use arags_proto::proto::index_chunk;
use arags_proto::proto::{
    GetCacheRequest, GetHistoryRequest, InvalidateCacheRequest, InvalidateMode, ListMemoryRequest,
    MemoryEntry, SearchRequest, SearchResult, TriggerMaintenanceRequest,
};

use crate::auth_client::AragsClient;
use crate::cli::Cli;
use crate::cli::commands::{Commands, MemoryCmd};
use crate::client::ClientConfig;
use crate::commands::persist::run_persist;
use crate::output::Format;
use crate::user_config::EffectiveUserConfig;

/// Connect to the server, performing `AuthRefresh` when a refresh token is
/// configured, and returning a client that auto-attaches the session token.
fn connect(rt: &Runtime, cfg: &EffectiveUserConfig) -> Result<AragsClient> {
    let client_config = ClientConfig {
        addr: cfg.server_addr(),
        tls_ca: cfg.server.tls_ca.clone(),
        tls_cert: cfg.server.tls_cert.clone(),
        tls_key: cfg.server.tls_key.clone(),
    };
    let auth = cfg.auth().cloned().unwrap_or_default();
    crate::auth_client::connect(rt, &client_config, &auth)
}

/// Map a textual tier (`fts`/`entity`/`vector`/`hybrid`/`auto`) onto the proto
/// enum. `auto` (and anything unknown) sends `UNSPECIFIED` so the server
/// applies its `[search].tier` default (plan 020).
fn map_search_tier(tier: &str) -> arags_proto::proto::SearchTier {
    debug!(tier, "resolving search tier");
    match tier {
        "fts" | "bm25" => arags_proto::proto::SearchTier::TierBm25,
        "entity" => arags_proto::proto::SearchTier::TierEntity,
        "vector" | "semantic" => arags_proto::proto::SearchTier::TierSemantic,
        "hybrid" => arags_proto::proto::SearchTier::TierHybrid,
        _ => arags_proto::proto::SearchTier::Unspecified,
    }
}

/// Entry point for the pure-gRPC dispatch.
#[instrument(skip(rt, cfg))]
pub fn run(
    cli: Cli,
    cfg: EffectiveUserConfig,
    project: PathBuf,
    format: Format,
    rt: &Runtime,
) -> Result<()> {
    match cli.command {
        Commands::Init { no_index, .. } => run_init(rt, &cfg, &project, format, !no_index),
        Commands::Index {
            path,
            ignore_patterns,
            force_include,
        } => {
            let mut client = connect(rt, &cfg)?;
            run_index(
                rt,
                &mut client,
                &project,
                &path,
                &ignore_patterns,
                &force_include,
                format,
            )
        }
        Commands::Search {
            query,
            top_k,
            tier,
            min_score,
            file_pattern,
            ..
        } => {
            let mut client = connect(rt, &cfg)?;
            run_search(
                rt,
                &mut client,
                &project,
                &query,
                top_k,
                &tier,
                min_score,
                file_pattern.as_deref(),
                format,
            )
        }
        Commands::Query {
            question,
            cache_id,
            qa,
            backend,
            model,
        } => {
            let mut client = connect(rt, &cfg)?;
            run_query(
                rt,
                &mut client,
                &project,
                &question,
                cache_id,
                qa,
                backend.as_deref(),
                model.as_deref(),
                format,
            )
        }
        Commands::Memory { cmd } => {
            let mut client = connect(rt, &cfg)?;
            run_memory(rt, &mut client, cmd, &project, format)
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
            run_history(rt, &mut client, &project, limit, user.as_deref(), format)
        }
    }
}

// ─────────────────────────────── Index ───────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_index(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &Path,
    path: &Path,
    ignore_patterns: &[String],
    force_include: &[String],
    format: Format,
) -> Result<()> {
    let absolute = std::fs::canonicalize(path)
        .with_context(|| format!("failed to resolve path: {}", path.display()))?;
    let project_str = project.to_string_lossy().to_string();

    // Combine CLI ignore patterns with the project's `.arags.toml` ignore list.
    let mut ignore = ignore_patterns.to_vec();
    ignore.extend(
        crate::user_config::load()
            .map(|c| c.ignore_patterns())
            .unwrap_or_default(),
    );

    let files = discover_files(&absolute, &ignore, force_include)
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
        let mut client = client.clone();
        let pb = progress.clone();
        let project = project_str.clone();
        let root = absolute.clone();
        let handle = rt
            .spawn(async move { stream_index_group(&mut client, project, root, group, pb).await });
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

/// Stream one disjoint group of files to the server as a single `IndexProject`
/// client-stream, returning the files/chunks counts reported by the server.
///
/// Each file's **raw text** is sent (the server chunks + embeds), per plan 020
/// D2.
async fn stream_index_group(
    client: &mut AragsClient,
    project: String,
    root: PathBuf,
    files: Vec<PathBuf>,
    progress: Arc<indicatif::ProgressBar>,
) -> anyhow::Result<(i64, i64)> {
    let (tx, rx) = mpsc::channel::<arags_proto::proto::IndexChunk>(32);
    let stream = ReceiverStream::new(rx);
    let response_fut = client.index_project(stream);

    let send_handle = tokio::spawn(async move {
        if tx
            .send(arags_proto::proto::IndexChunk {
                body: Some(index_chunk::Body::Init(arags_proto::proto::IndexInit {
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
            let size = i64::try_from(content.len()).unwrap_or(i64::MAX);
            if tx
                .send(arags_proto::proto::IndexChunk {
                    body: Some(index_chunk::Body::File(arags_proto::proto::IndexFile {
                        rel_path,
                        content: content.into_bytes(),
                        compressed: false,
                        size_bytes: size,
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

    let inner: arags_proto::proto::IndexResponse = response.into_inner();
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

/// Discover files under `root`, skipping default-ignored and user-ignored
/// paths unless force-included.
fn discover_files(
    root: &Path,
    ignore: &[String],
    force_include: &[String],
) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| anyhow::anyhow!("read dir entry failed: {e}"))?;
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_s = rel.to_string_lossy().to_string();
            let is_dir = path.is_dir();

            let forced = matches_any(&rel_s, force_include);
            let ignored = is_default_ignored(&rel_s, is_dir) || matches_any(&rel_s, ignore);

            if is_dir {
                if forced || !ignored {
                    stack.push(path);
                }
                continue;
            }
            if forced || !ignored {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// Directories/files ignored by default (sensitive or non-source).
fn is_default_ignored(rel: &str, is_dir: bool) -> bool {
    const DIRS: &[&str] = &[
        ".git",
        ".arags",
        "target",
        "node_modules",
        "vendor",
        ".venv",
        "venv",
        "__pycache__",
        ".idea",
        ".vscode",
        "dist",
        "build",
        ".next",
        ".terraform",
    ];
    const FILES: &[&str] = &[
        "*.lock", "*.png", "*.jpg", "*.jpeg", "*.gif", "*.ico", "*.pdf", "*.zip", "*.gz", "*.tar",
        "*.bin", "*.exe", "*.dll", "*.so", "*.dylib", "*.woff", "*.woff2", "*.ttf", "*.eot",
        "*.mp4", "*.mp3", "*.wav",
    ];
    if is_dir {
        DIRS.iter()
            .any(|d| rel == *d || rel.ends_with(&format!("/{d}")))
    } else {
        let rel_lc = rel.to_ascii_lowercase();
        FILES.iter().any(|f| rel_lc.ends_with(&f[1..])) // strip leading '*'
    }
}

/// Simple glob-ish matcher supporting `dir/`, `*.ext`, `*sub*`, and exact.
fn matches_any(rel: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| matches_pattern(rel, p))
}

fn matches_pattern(rel: &str, pat: &str) -> bool {
    if let Some(dir) = pat.strip_suffix('/') {
        return rel == dir
            || rel.starts_with(&format!("{dir}/"))
            || rel.contains(&format!("/{dir}/"));
    }
    if let Some(ext) = pat.strip_prefix("*.") {
        return rel.to_ascii_lowercase().ends_with(ext);
    }
    if pat.contains('*') {
        let simple = pat.replace('*', "");
        return !simple.is_empty() && rel.to_ascii_lowercase().contains(&simple);
    }
    rel == pat || rel.ends_with(&format!("/{pat}")) || rel.contains(&format!("/{pat}/"))
}

// ─────────────────────────────── Search ───────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_search(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &Path,
    query: &str,
    top_k: usize,
    tier: &str,
    min_score: Option<f32>,
    file_pattern: Option<&str>,
    format: Format,
) -> Result<()> {
    let project_str = project.to_string_lossy().to_string();
    let request = Request::new(SearchRequest {
        project: project_str,
        query: query.to_string(),
        max_results: top_k as i32,
        tier: map_search_tier(tier) as i32,
    });
    let response = rt.block_on(client.search(request))?;
    let mut results = response.into_inner().results;

    if let Some(min) = min_score {
        results.retain(|r| r.score >= min);
    }
    if let Some(pat) = file_pattern {
        results.retain(|r| r.file_path.contains(pat));
    }

    let rendered = render_search(&results, query, format);
    print!("{rendered}");
    Ok(())
}

fn render_search(results: &[SearchResult], query: &str, format: Format) -> String {
    match format {
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
            crate::output::jsonl::render_content_jsonl("query", query, &pairs)
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
    }
}

// ─────────────────────────────── Query ───────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_query(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &Path,
    question: &str,
    cache_id: Option<String>,
    qa: bool,
    backend: Option<&str>,
    model: Option<&str>,
    format: Format,
) -> Result<()> {
    let project_str = project.to_string_lossy().to_string();

    if let Some(id) = cache_id {
        return crate::commands::qa_cache::run_get(rt, client, &id, &project_str, format);
    }
    if qa {
        return crate::commands::qa_cache::run_ask(
            rt,
            client,
            question,
            backend,
            model,
            &project_str,
            format,
        );
    }

    // Default: server-side context (no client LLM), deterministic. Mirrors the
    // removed `context` command.
    let request = Request::new(arags_proto::proto::ContextRequest {
        project: project_str.clone(),
        task: question.to_string(),
        ..Default::default()
    });
    let response = rt.block_on(client.build_context(request))?;
    let ctx = response.into_inner().context;
    let rendered = match format {
        Format::FullJson => crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({ "question": question, "context": ctx }))
            .to_json_string(),
        Format::Jsonl => {
            let pairs: Vec<(String, String)> = vec![(project_str.clone(), ctx.clone())];
            crate::output::jsonl::render_content_jsonl("question", question, &pairs)
        }
        _ => ctx,
    };
    print!("{rendered}");
    Ok(())
}

// ─────────────────────────────── Memory ───────────────────────────────

fn run_memory(
    rt: &Runtime,
    client: &mut AragsClient,
    cmd: MemoryCmd,
    _project: &Path,
    format: Format,
) -> Result<()> {
    match cmd {
        MemoryCmd::List {
            project,
            limit,
            include_entities,
        } => {
            let request = Request::new(ListMemoryRequest {
                project: project.unwrap_or_default(),
                limit,
                include_entities,
            });
            let resp = rt.block_on(client.list_memory(request))?.into_inner();
            render_memory_list(&resp.entries, &resp.stats, format);
        }
        MemoryCmd::Get { cache_id } => {
            let request = Request::new(GetCacheRequest { cache_id });
            let resp = rt.block_on(client.get_cache(request))?.into_inner();
            render_cache_get(&resp, format);
        }
        MemoryCmd::Invalidate {
            cache_id,
            project,
            delete,
            radius,
            ..
        } => {
            let request = Request::new(InvalidateCacheRequest {
                project: project.unwrap_or_default(),
                cache_id: cache_id.unwrap_or_default(),
                mode: if delete {
                    InvalidateMode::Delete as i32
                } else {
                    InvalidateMode::Stale as i32
                },
                similarity_radius: radius.unwrap_or(0.0),
            });
            let resp = rt.block_on(client.invalidate_cache(request))?.into_inner();
            println!(
                "invalidated {} cache entr(y/ies) by {}",
                resp.invalidated, resp.invalidated_by
            );
        }
        MemoryCmd::Cleanup { dry_run, project } => {
            let request = Request::new(TriggerMaintenanceRequest {
                project: project.unwrap_or_default(),
                dry_run,
            });
            let resp = rt
                .block_on(client.trigger_maintenance(request))?
                .into_inner();
            println!(
                "maintenance complete (dry_run={dry_run}): {} duplicate chunks removed, \
                 {} low-confidence patterns removed, {} chunks decayed, {} kept",
                resp.duplicate_chunks_removed,
                resp.low_confidence_patterns_removed,
                resp.decayed_chunks,
                resp.kept
            );
        }
    }
    Ok(())
}

fn render_memory_list(entries: &[MemoryEntry], stats: &str, format: Format) {
    if format == Format::FullJson {
        let items: Vec<serde_json::Value> = entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "cache_id": e.cache_id,
                    "project": e.project,
                    "question": e.question,
                    "created_at": e.created_at,
                    "score": e.score,
                    "entities": e.entities,
                })
            })
            .collect();
        let out = crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({ "entries": items, "stats": stats }))
            .to_json_string();
        print!("{out}");
    } else {
        if entries.is_empty() {
            println!("No cached memory.");
            return;
        }
        for e in entries {
            println!(
                "{}  [{}]  {}  (score {:.3})",
                e.cache_id, e.project, e.question, e.score
            );
        }
        if !stats.is_empty() {
            println!("\nstats: {stats}");
        }
    }
}

fn render_cache_get(resp: &arags_proto::proto::GetCacheResponse, format: Format) {
    if format == Format::FullJson {
        let out = crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({
                "project": resp.project,
                "answer": resp.answer,
                "source_chunk_ids": resp.source_chunk_ids,
                "files": resp.files,
            }))
            .to_json_string();
        print!("{out}");
    } else {
        println!("Project: {}", resp.project);
        println!("Files: {}", resp.files.join(", "));
        println!("Source chunks: {}", resp.source_chunk_ids.join(", "));
        println!("\n{}\n", resp.answer);
    }
}

// ─────────────────────────────── History ───────────────────────────────

fn run_history(
    rt: &Runtime,
    client: &mut AragsClient,
    _project: &Path,
    limit: usize,
    user: Option<&str>,
    format: Format,
) -> Result<()> {
    let request = Request::new(GetHistoryRequest {
        user: user.unwrap_or_default().to_string(),
        limit: limit as i64,
    });
    let resp = rt.block_on(client.get_history(request))?.into_inner();
    if format == Format::FullJson {
        let items: Vec<serde_json::Value> = resp
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "user": e.user,
                    "question": e.question,
                    "created_at": e.created_at,
                    "cache_id": e.cache_id,
                })
            })
            .collect();
        let out = crate::output::json::JsonOutput::ok()
            .with_data(serde_json::json!({ "entries": items, "count": items.len() }))
            .to_json_string();
        print!("{out}");
    } else {
        if resp.entries.is_empty() {
            println!("No history found.");
            return Ok(());
        }
        for e in &resp.entries {
            println!(
                "[{}] {} — {} (cache: {})",
                e.created_at, e.user, e.question, e.cache_id
            );
        }
    }
    Ok(())
}

// ─────────────────────────────── Init ───────────────────────────────

fn run_init(
    rt: &Runtime,
    cfg: &EffectiveUserConfig,
    project: &Path,
    format: Format,
    do_index: bool,
) -> Result<()> {
    // Validate global identity (auth). The refresh token lives only in the
    // global `~/.arags/arags.toml`; we never copy it into the local file.
    match cfg.auth() {
        Some(auth) if auth.refresh_token.is_some() => {}
        _ => {
            bail!(
                "no global identity configured. Run `arags-server admin create-refresh` and \
                 store the token in `~/.arags/arags.toml` under `[auth]`."
            );
        }
    }

    let project_name = project_name(project);
    let local_path = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".arags.toml");

    if local_path.exists() {
        println!(
            "{} already exists; leaving it untouched.",
            local_path.display()
        );
    } else {
        let ignore = seed_ignore_from_gitignore();
        // No `[server]` section on purpose (agnostic-rlm-rs-152a): a
        // hardcoded localhost stamp would override the operator's global
        // `~/.arags/arags.toml` in the field-by-field merge. Absent here, the
        // merge falls back to the global addr (default `127.0.0.1:50051`).
        let content = toml::to_string_pretty(&LocalAragsToml {
            project: LocalProject {
                name: project_name.clone(),
                ignore: if ignore.is_empty() {
                    None
                } else {
                    Some(ignore)
                },
            },
        })
        .context("failed to serialize .arags.toml")?;
        std::fs::write(&local_path, content)
            .with_context(|| format!("failed to write {}", local_path.display()))?;
        println!("Created {}", local_path.display());
        append_gitignore(&local_path)?;
    }

    if do_index {
        let mut client = connect(rt, cfg)?;
        run_index(rt, &mut client, project, project, &[], &[], format)?;
    } else {
        println!("Skipping index (--no-index). Run `arags index` to ingest.");
    }
    Ok(())
}

/// Local `.arags.toml` shape written by `arags init`.
#[derive(serde::Serialize)]
struct LocalAragsToml {
    project: LocalProject,
}

#[derive(serde::Serialize)]
struct LocalProject {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ignore: Option<Vec<String>>,
}

/// Best-effort project name: git remote, else directory basename.
fn project_name(project: &Path) -> String {
    if let Ok(output) = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project)
        .output()
    {
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout);
            if let Some(name) = url
                .trim()
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".git"))
            {
                if !name.is_empty() {
                    return name.to_string();
                }
            }
        }
    }
    project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

/// Seed ignore patterns from the project's `.gitignore`, if present.
fn seed_ignore_from_gitignore() -> Vec<String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gitignore = cwd.join(".gitignore");
    let Ok(content) = std::fs::read_to_string(&gitignore) else {
        return vec![
            ".git/".to_string(),
            "target/".to_string(),
            "node_modules/".to_string(),
        ];
    };
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Append `.arags.toml` to `.gitignore` (idempotent).
fn append_gitignore(local_path: &Path) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let gitignore = cwd.join(".gitignore");
    let entry = local_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".arags.toml");
    if let Ok(existing) = std::fs::read_to_string(&gitignore) {
        if existing.lines().any(|l| l.trim() == entry) {
            return Ok(());
        }
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&gitignore)
        .with_context(|| format!("failed to open {}", gitignore.display()))?;
    writeln!(f, "{entry}").context("failed to append to .gitignore")?;
    Ok(())
}
