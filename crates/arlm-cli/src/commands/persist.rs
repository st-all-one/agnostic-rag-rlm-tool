//! `arlm persist <response_id>` (plan 019 D / 020).
//!
//! Flow: print the `response_id`, fetch the served answer via `GetAnswerById`
//! (server), synthesize a structured wiki article with the **user's** LLM
//! (same backend used by `query -qa`), and write it under `wiki/` in the
//! project. No git operations are performed.

use std::path::Path;

use anyhow::{Context, Result, bail};

use tokio::runtime::Runtime;

use arlm_llm::{CompletionRequest, Message, Role};
use chrono::Utc;

use arlm_proto::proto::GetAnswerByIdRequest;

use crate::auth_client::ArlmClient;
use crate::output::Format;
use crate::user_config::EffectiveUserConfig;

/// Persist a served answer as a structured wiki page.
#[allow(clippy::too_many_arguments)]
pub fn run_persist(
    rt: &Runtime,
    client: &mut ArlmClient,
    cfg: &EffectiveUserConfig,
    project: &Path,
    response_id: &str,
    title: Option<&str>,
    format: Format,
) -> Result<()> {
    println!("Response ID: {response_id}");

    let resp = rt
        .block_on(client.get_answer_by_id(GetAnswerByIdRequest {
            cache_id: response_id.to_string(),
            project: String::new(),
        }))?
        .into_inner();

    if !resp.found {
        bail!("answer {response_id} not found for this project");
    }

    let answer_text = resp.answer_text;
    let source_chunk_ids = resp.source_chunk_ids;
    let source_hashes = resp.source_hashes;

    // Resolve the user's LLM (must be configured in ~/.arlm/arlm.toml).
    let llm_config = cfg
        .llm_config()
        .cloned()
        .context("no [llm] configured; add a backend to ~/.arlm/arlm.toml")?;
    let model = llm_config
        .backends
        .first()
        .and_then(|b| b.model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let backend = crate::backend::resolve_backend(Some(&llm_config), None, None)
        .context("failed to build LLM backend for summarization")?;

    let provenance = build_provenance(&source_chunk_ids, &source_hashes);
    let prompt = format!(
        "You are a technical writer maintaining a project knowledge base. \
         Below is an answer previously produced by a query-answer system, along \
         with its provenance (source chunk ids and content hashes).\n\n\
         ANSWER:\n{answer_text}\n\nPROVENANCE:\n{provenance}\n\n\
         Rewrite this into a clean, structured knowledge-base article. \
         Use exactly these top-level sections, in this order, with no extra \
         preamble:\n## Summary\n## Key Findings / Artifacts\n## Related\n"
    );

    let summary = rt
        .block_on(backend.complete(CompletionRequest {
            model: model.clone(),
            messages: vec![Message {
                role: Role::User,
                content: prompt,
            }],
            temperature: Some(0.3),
            max_tokens: Some(2048),
            stop: None,
            seed: None,
            tools: None,
        }))
        .context("LLM summarization failed")?
        .content;

    let generated = render_wiki(
        response_id,
        &model,
        &project_name(project),
        &provenance,
        &summary,
        title,
    );

    let wiki_dir = project.join("wiki");
    std::fs::create_dir_all(&wiki_dir)
        .with_context(|| format!("failed to create {}", wiki_dir.display()))?;
    let slug = slugify(title.or(Some(summary.as_str())));
    let filename = format!("{}_{}.md", Utc::now().format("%Y%m%d%H%M"), slug);
    let path = wiki_dir.join(&filename);
    std::fs::write(&path, generated)
        .with_context(|| format!("failed to write {}", path.display()))?;

    match format {
        Format::FullJson => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "response_id": response_id,
                    "path": path.to_string_lossy(),
                    "model": model,
                }))?
            );
        }
        _ => {
            println!("Persisted to: {}", path.display());
        }
    }
    Ok(())
}

/// Render the fixed-structure wiki document.
fn render_wiki(
    response_id: &str,
    model: &str,
    project: &str,
    provenance: &str,
    summary: &str,
    _title: Option<&str>,
) -> String {
    format!(
        "# Persisted Answer\n\n\
         - **Response ID:** {response_id}\n\
         - **Generated:** {generated}\n\
         - **Model:** {model}\n\
         - **Project:** {project}\n\
         - **Provenance:** {provenance}\n\n\
         {summary}\n",
        response_id = response_id,
        generated = Utc::now().format("%Y%m%d%H%M"),
        model = model,
        project = project,
        provenance = provenance,
        summary = summary,
    )
}

fn build_provenance(chunk_ids: &[String], hashes: &[String]) -> String {
    let ids = if chunk_ids.is_empty() {
        "(none)".to_string()
    } else {
        chunk_ids.join(", ")
    };
    let hashes = if hashes.is_empty() {
        "(none)".to_string()
    } else {
        hashes.join(", ")
    };
    format!("chunk_ids: {ids}\nhashes: {hashes}")
}

fn project_name(project: &Path) -> String {
    project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

/// Produce a filesystem-safe slug from a title/summary.
fn slugify(input: Option<&str>) -> String {
    let Some(input) = input else {
        return "untitled".to_string();
    };
    let trimmed: String = input
        .split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = trimmed.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed
    }
}
