//! `arags persist <response_id>` (plan 019 D / 020).
//!
//! Flow: print the `response_id`, fetch the served answer via `GetAnswerById`
//! (server), synthesize a structured wiki article with the **user's** LLM
//! (same backend used by `query -qa`), and write it under `wiki/` in the
//! project. No git operations are performed.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use tokio::runtime::Runtime;
use tracing::debug;

use arags_llm::{CompletionRequest, LlmBackend, Message, Role};
use chrono::Utc;

use arags_proto::proto::GetAnswerByIdRequest;

use crate::auth_client::AragsClient;
use crate::output::Format;
use crate::user_config::EffectiveUserConfig;

/// Persist a served answer as a structured wiki page.
#[allow(clippy::too_many_arguments)]
pub fn run_persist(
    rt: &Runtime,
    client: &mut AragsClient,
    cfg: &EffectiveUserConfig,
    project: &Path,
    response_id: &str,
    title: Option<&str>,
    format: Format,
) -> Result<()> {
    println!("Response ID: {response_id}");

    // Resolve the canonical project name so the server can locate the cached
    // answer (answers are scoped per project). The local `.arags.toml`
    // `[project].name` is authoritative; fall back to the path only if unset.
    let project_name = cfg
        .project
        .name
        .clone()
        .unwrap_or_else(|| project.to_string_lossy().to_string());

    let resp = rt
        .block_on(client.get_answer_by_id(GetAnswerByIdRequest {
            cache_id: response_id.to_string(),
            project: project_name,
        }))?
        .into_inner();

    if !resp.found {
        bail!("answer {response_id} not found for this project");
    }

    let answer_text = resp.answer_text;
    let source_chunk_ids = resp.source_chunk_ids;
    let source_hashes = resp.source_hashes;

    // Resolve the user's LLM (must be configured in ~/.arags/arags.toml).
    let llm_config = cfg
        .llm_config()
        .cloned()
        .context("no [llm] configured; add a backend to ~/.arags/arags.toml")?;
    let model = llm_config
        .backends
        .first()
        .and_then(|b| b.model.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let backend = crate::backend::resolve_backend(Some(&llm_config), None, None)
        .context("failed to build LLM backend for summarization")?;

    let provenance = build_provenance(&source_chunk_ids, &source_hashes);
    let project_name = project.to_string_lossy();
    let summary = generate_summary(
        rt,
        backend.as_ref(),
        &project_name,
        &answer_text,
        &provenance,
        &model,
    )?;

    let _path = write_wiki(
        project,
        response_id,
        &model,
        &provenance,
        &summary,
        title,
        format,
    )?;
    Ok(())
}

/// Build the summary completion request from the served answer + provenance,
/// call the user's LLM, strip any leaked chain-of-thought, and return the
/// cleaned summary text.
///
/// Extracted from [`run_persist`] so the client-side summarize path is
/// unit-testable without a live gRPC server or real LLM. Timing of the LLM call
/// is recorded via `tracing::debug!` (`elapsed_ms` + `model`).
pub(crate) fn generate_summary(
    rt: &Runtime,
    backend: &dyn LlmBackend,
    project_name: &str,
    answer_text: &str,
    provenance: &str,
    model: &str,
) -> Result<String> {
    let prompt = crate::prompts::build_summarize_prompt(
        crate::prompts::SummarizeScope::Project,
        project_name,
        answer_text,
        Some(provenance),
    );

    let start = std::time::Instant::now();
    let response = rt
        .block_on(backend.complete(CompletionRequest {
            model: model.to_string(),
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
        .context("LLM summarization failed")?;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    debug!(elapsed_ms, model, "llm summarize call complete");

    let summary = response.content;
    let stripped = crate::llm_post::strip_cot(&summary);
    if stripped.len() != summary.len() {
        debug!(
            chars_removed = summary.len().saturating_sub(stripped.len()),
            "stripped chain-of-thought from summary"
        );
    }
    Ok(stripped)
}

/// Render and persist a wiki page under `<project>/wiki/`.
///
/// Renders the fixed-structure document via [`render_wiki`], writes it to a
/// timestamped `.md` file, prints the resulting path (JSON when `format ==
/// FullJson`), and returns the written path. Extracted from [`run_persist`] so
/// the storage side is unit-testable without a live gRPC server.
pub(crate) fn write_wiki(
    project: &Path,
    response_id: &str,
    model: &str,
    provenance: &str,
    summary: &str,
    title: Option<&str>,
    format: Format,
) -> Result<PathBuf> {
    let generated = render_wiki(
        response_id,
        model,
        &project_name(project),
        provenance,
        summary,
        title,
    );

    let wiki_dir = project.join("wiki");
    std::fs::create_dir_all(&wiki_dir)
        .with_context(|| format!("failed to create {}", wiki_dir.display()))?;
    let slug = slugify(title.or(Some(summary)));
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
    Ok(path)
}

/// Render the fixed-structure wiki document.
pub(crate) fn render_wiki(
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{MockLlmBackend, clean_summary_reply, cot_leak_reply};
    use tempfile::TempDir;

    fn test_rt() -> Runtime {
        Runtime::new().expect("failed to build tokio runtime for test")
    }

    #[test]
    fn generate_summary_strips_cot_and_has_sections() {
        let rt = test_rt();
        let backend = MockLlmBackend::new(cot_leak_reply());
        let summary = generate_summary(
            &rt,
            &backend,
            "proj",
            "The answer text from the server.",
            "chunk_ids: c1\nhashes: h1",
            "mock",
        )
        .expect("generate_summary should succeed");

        assert!(
            !summary.contains("<think>"),
            "leaked chain-of-thought must be stripped"
        );
        assert!(
            !summary.contains("leaked reasoning"),
            "CoT content must not survive stripping"
        );
        assert!(
            summary.contains("## Summary"),
            "prompt mandates a ## Summary section; got:\n{summary}"
        );
    }

    #[test]
    fn render_wiki_includes_summary() {
        let summary = "## Summary\n\nA concise summary.\n\n## Key Findings / Artifacts\n\n- x";
        let rendered = render_wiki("resp-1", "mock", "proj", "chunk_ids: c1", summary, None);
        assert!(rendered.contains(summary), "wiki must embed the summary");
        assert!(rendered.contains("resp-1"));
        assert!(rendered.contains("proj"));
    }

    #[test]
    fn write_wiki_creates_md_file_under_wiki_dir() {
        let rt = test_rt();
        let tmp = TempDir::new().unwrap();
        let project = tmp.path();

        let summary = generate_summary(
            &rt,
            &MockLlmBackend::new(clean_summary_reply()),
            "proj",
            "answer",
            "chunk_ids: c1",
            "mock",
        )
        .unwrap();

        let path = write_wiki(
            project,
            "resp-9",
            "mock",
            "chunk_ids: c1",
            &summary,
            Some("My Title"),
            Format::Markdown,
        )
        .expect("write_wiki should succeed");

        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("md"));
        assert_eq!(path.parent(), Some(project.join("wiki").as_path()));
        let written = std::fs::read_to_string(&path).expect("file should be readable");
        assert!(written.contains(&summary), "file must contain the summary");
        assert!(written.contains("resp-9"));
    }

    #[test]
    fn write_wiki_fulljson_prints_path() {
        let rt = test_rt();
        let tmp = TempDir::new().unwrap();
        let summary = generate_summary(
            &rt,
            &MockLlmBackend::new(clean_summary_reply()),
            "proj",
            "answer",
            "chunk_ids: c1",
            "mock",
        )
        .unwrap();
        let path = write_wiki(
            tmp.path(),
            "resp-json",
            "mock",
            "chunk_ids: c1",
            &summary,
            None,
            Format::FullJson,
        )
        .unwrap();
        assert!(path.exists());
    }
}
