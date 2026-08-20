use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::{data_dir, project_name};

pub struct QueryConfig<'a> {
    pub question: &'a str,
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
    pub llm: bool,
}

pub async fn execute(config: QueryConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_query");

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Querying: {}", config.question));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    // Search for relevant context
    let context_str = if let Ok(Some(buffer)) = storage.get_buffer_by_name(&pname) {
        let bm25 =
            arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
        let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
        let options = arlm_search::SearchOptions {
            tier: arlm_search::SearchTier::Entity,
            top_k: 10,
        };
        let results = hybrid
            .search(
                config.question,
                None,
                buffer.id,
                &options,
                None,
                Some(&storage),
            )
            .await
            .unwrap_or_default();
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Without --llm: return deterministic search results as context
    if !config.llm {
        match config.format {
            Format::Json => {
                let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                    "question": config.question,
                    "context": context_str,
                    "project": pname,
                    "llm": false,
                }));
                output.print();
            }
            Format::Tree => {
                output::success(&format!("Context for: {}", config.question));
                println!("\n{context_str}");
            }
            Format::Markdown => {
                println!("## {}\n\n{context_str}", config.question);
            }
            Format::Prompt => {
                println!("{context_str}");
            }
        }
        return Ok(());
    }

    // With --llm: call LLM with context
    let backend_name = config.backend.unwrap_or("ollama");
    let kind: arlm_llm::BackendKind = backend_name.parse().context("failed to parse backend")?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
        arlm_llm::BackendKind::DeepSeek => "DEEPSEEK_API_KEY",
        arlm_llm::BackendKind::MiMo => "MIMO_API_KEY",
    })
    .ok();

    let llm_backend =
        arlm_llm::get_backend(&kind, api_key, None).context("failed to create LLM backend")?;

    let prompt = format!(
        "Based on the following project context, answer this question:\n\nQuestion: {}\n\nContext:\n{context_str}",
        config.question
    );

    let response = llm_backend
        .complete(arlm_llm::CompletionRequest {
            model: config.model.unwrap_or("llama3").to_string(),
            messages: vec![arlm_llm::Message {
                role: arlm_llm::Role::User,
                content: prompt,
            }],
            temperature: Some(0.7),
            max_tokens: Some(2048),
            stop: None,
        })
        .await
        .context("LLM completion failed")?;

    match config.format {
        Format::Json => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "question": config.question,
                "answer": response.content,
                "model": response.model,
                "llm": true,
            }));
            output.print();
        }
        Format::Tree => {
            output::success(&format!("Answer for: {}", config.question));
            println!("\n{}", response.content);
        }
        Format::Markdown => {
            println!("## {}\n\n{}", config.question, response.content);
        }
        Format::Prompt => {
            println!("{}", response.content);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_query_no_project() {
        let tmp = TempDir::new().unwrap();
        let project_path = tmp.path().join("nonexistent");
        // Without --llm, query succeeds even if project doesn't exist (returns empty context)
        let config = QueryConfig {
            question: "what is auth?",
            backend: Some("ollama"),
            model: None,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
            llm: false,
        };
        let result = execute(config).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_query_with_llm_no_project() {
        let tmp = TempDir::new().unwrap();
        let project_path = tmp.path().join("nonexistent");
        // With --llm but no backend configured, should fail
        let config = QueryConfig {
            question: "what is auth?",
            backend: Some("ollama"),
            model: None,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
            llm: true,
        };
        let result = execute(config).await;
        assert!(result.is_err());
    }
}
