use std::path::Path;

use anyhow::{Context, Result};

use crate::output::{self, Format};
use crate::util::project_dirs;

pub struct QueryConfig<'a> {
    pub question: &'a str,
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
}

pub async fn execute(config: QueryConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_query");

    let project_name = config
        .project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let backend_name = config.backend.unwrap_or("ollama");
    let kind: arlm_llm::BackendKind = backend_name.parse().context("failed to parse backend")?;

    let api_key = std::env::var(match kind {
        arlm_llm::BackendKind::OpenAI => "OPENAI_API_KEY",
        arlm_llm::BackendKind::Anthropic => "ANTHROPIC_API_KEY",
        arlm_llm::BackendKind::Gemini => "GEMINI_API_KEY",
        arlm_llm::BackendKind::Ollama => "",
    })
    .ok();

    let llm_backend =
        arlm_llm::get_backend(&kind, api_key, None).context("failed to create LLM backend")?;

    if config.verbose {
        output::info(&format!("Querying: {}", config.question));
    }

    let data_dir = project_dirs().join(project_name);
    let storage = arlm_storage::Storage::open(&data_dir).context("failed to open storage")?;

    let context_str = if let Ok(Some(buffer)) = storage.get_buffer_by_name(project_name) {
        let bm25 =
            arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
        let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
        let results = hybrid
            .search_fts(config.question, buffer.id, 10, None)
            .unwrap_or_default();
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt)
            .unwrap_or_default()
    } else {
        String::new()
    };

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
                "usage": {
                    "prompt_tokens": response.usage.prompt_tokens,
                    "completion_tokens": response.usage.completion_tokens,
                    "total_tokens": response.usage.total_tokens,
                },
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
        let config = QueryConfig {
            question: "what is auth?",
            backend: Some("ollama"),
            model: None,
            project: project_path.as_path(),
            format: Format::Json,
            verbose: false,
        };
        let result = execute(config).await;
        assert!(result.is_err());
    }
}
