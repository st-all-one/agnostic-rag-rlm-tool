//! `arags explore` RPCs, contract parsing and multi-format rendering (plan 022).
//!
//! The persist path validates the EXPLORATIONS.md contract locally before
//! hitting the server so agents get fast, precise feedback: a header block
//! (`goal:`/`files:`) plus the four fixed sections. Parsing helpers are pure
//! functions; all I/O stays in [`run_explore_persist`].

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use arags_proto::proto::{PersistExplorationRequest, SearchExplorationsRequest};
use tokio::runtime::Runtime;
use tonic::Request;

#[cfg(test)]
mod tests;

use crate::auth_client::AragsClient;
use crate::cli::commands::ExploreCmd;
use crate::output::Format;

/// A parsed exploration contract document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Contract {
    /// Objective that drove the exploration (`goal:` header).
    pub goal: String,
    /// Short digest for embedding (`summary:` header, else first paragraph
    /// of the Mapa section).
    pub summary: String,
    /// Cited file paths (`files:` header, comma-separated).
    pub files: Vec<String>,
    /// LLM model (`model:` header; optional).
    pub model: String,
    /// The untouched markdown document sent as body.
    pub body_markdown: String,
}

/// Parse and validate an EXPLORATIONS.md contract document.
///
/// # Errors
///
/// Returns a human-readable error when the header or any required section is
/// missing/empty.
pub(crate) fn parse_contract(content: &str) -> std::result::Result<Contract, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("contract document is empty".into());
    }
    if !trimmed.starts_with("---") {
        return Err("missing header block: document must start with '---'".into());
    }

    let rest = &trimmed[3..];
    let Some(header_end) = rest.find("---") else {
        return Err("unterminated header block: closing '---' not found".into());
    };
    let header = &rest[..header_end];
    let body_markdown = rest[header_end + 3..].trim().to_string();

    let mut goal = String::new();
    let mut summary = String::new();
    let mut files = Vec::new();
    let mut model = String::new();
    for line in header.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "goal" => goal = value.to_string(),
            "summary" => summary = value.to_string(),
            "model" => model = value.to_string(),
            "files" => {
                files = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
            }
            _ => {}
        }
    }

    if goal.is_empty() {
        return Err("header 'goal:' is required".into());
    }
    if files.is_empty() {
        return Err("header 'files:' is required (anchors keep the map honest)".into());
    }
    for section in ["## Mapa", "## Conexões", "## Evidências", "## Limitações"] {
        if !body_markdown.contains(section) {
            return Err(format!("missing required section '{section}'"));
        }
    }
    if summary.is_empty() {
        // First non-empty paragraph after the Mapa heading.
        summary = body_markdown
            .split_once("## Mapa")
            .and_then(|(_, tail)| tail.lines().map(str::trim).find(|l| !l.is_empty()))
            .unwrap_or_default()
            .to_string();
        if summary.is_empty() {
            return Err("'## Mapa' section has no content to summarize".into());
        }
    }

    Ok(Contract {
        goal,
        summary,
        files,
        model,
        body_markdown,
    })
}

/// Dispatch `arags explore` subcommands.
pub(crate) fn run_explore(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &str,
    cmd: ExploreCmd,
    format: Format,
) -> Result<()> {
    match cmd {
        ExploreCmd::Search {
            query,
            project: explicit,
            limit,
            include_stale,
            as_of_epoch,
            as_of,
        } => {
            let scope = explicit.unwrap_or_else(|| project.to_string());
            let epoch = crate::cli::commands::resolve_as_of_epoch(as_of_epoch, as_of)?;
            run_explore_search(
                rt,
                client,
                &scope,
                &query,
                limit,
                include_stale,
                epoch,
                format,
            )
        }
        ExploreCmd::Persist { map, paths } => {
            run_explore_persist(rt, client, project, &map, &paths)
        }
    }
}

fn run_explore_search(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &str,
    query: &str,
    limit: i32,
    include_stale: bool,
    as_of_epoch: i64,
    format: Format,
) -> Result<()> {
    let request = Request::new(SearchExplorationsRequest {
        project: project.to_string(),
        query: query.to_string(),
        limit,
        include_stale,
        as_of_epoch,
    });
    let response = rt
        .block_on(client.search_explorations(request))
        .context("search explorations failed")?
        .into_inner();

    let rendered = render_hits(&response.hits, query, as_of_epoch, format);
    print!("{rendered}");
    Ok(())
}

fn run_explore_persist(
    rt: &Runtime,
    client: &mut AragsClient,
    project: &str,
    map: &PathBuf,
    extra_paths: &[String],
) -> Result<()> {
    let content = if map.as_os_str() == "-" {
        use std::io::Read as _;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("failed to read contract from stdin")?;
        buf
    } else {
        std::fs::read_to_string(map).with_context(|| format!("failed to read {}", map.display()))?
    };

    let contract = parse_contract(&content).map_err(anyhow::Error::msg)?;
    let mut files = contract.files.clone();
    for p in extra_paths {
        if !files.contains(p) {
            files.push(p.clone());
        }
    }

    let request = Request::new(PersistExplorationRequest {
        project: project.to_string(),
        goal: contract.goal,
        summary: contract.summary,
        body_markdown: contract.body_markdown,
        files,
        created_by: String::new(),
        model: contract.model,
    });
    let resp = rt
        .block_on(client.persist_exploration(request))
        .context("persist exploration failed")?
        .into_inner();
    if !resp.accepted {
        anyhow::bail!("server rejected the map: {}", resp.reason);
    }
    println!("persisted {}", resp.exploration_id);
    for path in &resp.unresolved_paths {
        println!("warning: path not in index (not anchored): {path}");
    }
    Ok(())
}

fn render_hits(
    hits: &[arags_proto::proto::ExplorationHit],
    query: &str,
    as_of_epoch: i64,
    format: Format,
) -> String {
    if format == Format::FullJson {
        let items: Vec<serde_json::Value> = hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "exploration_id": h.exploration_id,
                    "goal": h.goal,
                    "summary": h.summary,
                    "confidence": h.confidence,
                    "similarity": h.similarity,
                    "status": h.status,
                    "stale_reason": h.stale_reason,
                    "epoch_drift": h.epoch_drift,
                    "confirmed": h.confirmed,
                    "contradicted": h.contradicted,
                    "created_by": h.created_by,
                    "model": h.model,
                })
            })
            .collect();
        return serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "as_of_epoch": if as_of_epoch > 0 { Some(as_of_epoch) } else { None },
            "hits": items,
        }))
        .unwrap_or_else(|_| "[]".into());
    }

    if hits.is_empty() {
        return format!("no exploration maps for \"{query}\"\n");
    }
    let mut out = String::new();
    if as_of_epoch > 0 {
        let _ = writeln!(out, "> **Time-travel snapshot** at epoch `{as_of_epoch}`\n");
    }
    for h in hits {
        let _ = writeln!(
            out,
            "# {} [{}] confidence={:.2} similarity={:.2}",
            h.goal, h.status, h.confidence, h.similarity
        );
        if !h.stale_reason.is_empty() {
            let _ = writeln!(out, "  stale: {}", h.stale_reason.join(", "));
        }
        let _ = writeln!(
            out,
            "  by {} via {} | confirms={} contradicts={}",
            h.created_by, h.model, h.confirmed, h.contradicted
        );
        let _ = writeln!(
            out,
            "  id: {}\n  summary: {}\n",
            h.exploration_id, h.summary
        );
    }
    out
}
