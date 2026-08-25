//! Memory/cache admin and history RPCs.

use std::path::Path;

use anyhow::Result;
use tokio::runtime::Runtime;
use tonic::Request;

use arags_proto::proto::{
    GetCacheRequest, GetHistoryRequest, InvalidateCacheRequest, InvalidateMode, ListMemoryRequest,
    MemoryEntry, TriggerMaintenanceRequest,
};

use crate::auth_client::AragsClient;
use crate::cli::commands::MemoryCmd;
use crate::output::Format;

pub(crate) fn run_memory(
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

pub(crate) fn run_history(
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
