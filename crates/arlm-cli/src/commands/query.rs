use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::backend::resolve_backend;
use crate::config::Config;
use crate::embedding::{build_embedder_from_config, open_vector_store};
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
    pub config: &'a Config,
}

pub async fn execute(config: QueryConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_query");

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Querying: {}", config.question));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    // Search for relevant context. Semantic (BGE-M3) filtering is wired in: we
    // build a real embedder, open the per-buffer vector store, and pass a query
    // vector so the `Vector` tier actually runs (previously `semantic = None`
    // and `query_vector = None`, so semantic search never executed).
    let (context_str, query_results) = if let Ok(Some(buffer)) = storage.get_buffer_by_name(&pname)
    {
        let bm25 =
            arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;

        let embedder = build_embedder_from_config(config.config, "search_query: ");
        let (semantic, query_vector) = match open_vector_store(buffer.id, embedder.dimensions())
            .await
        {
            Ok(vstore) => {
                if vstore.dimensions() == embedder.dimensions() {
                    match embedder.embed(config.question) {
                        Ok(vec) => {
                            let sem = arlm_search::SemanticSearch::new(Arc::clone(&vstore));
                            (Some(sem), Some(vec))
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "query embedding failed, semantic tier disabled");
                            (None, None)
                        }
                    }
                } else {
                    tracing::warn!(
                        store_dims = vstore.dimensions(),
                        embedder_dims = embedder.dimensions(),
                        "vector store dimensionality mismatch, semantic tier disabled"
                    );
                    (None, None)
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "vector store unavailable, semantic tier disabled");
                (None, None)
            }
        };

        let hybrid = arlm_search::HybridSearch::new(bm25, None, semantic);
        let tier = if query_vector.is_some() {
            arlm_search::SearchTier::Vector
        } else {
            arlm_search::SearchTier::Entity
        };
        let options = arlm_search::SearchOptions { tier, top_k: 15 };
        let results = hybrid
            .search(
                config.question,
                query_vector.as_deref(),
                buffer.id,
                &options,
                None,
                Some(&storage),
            )
            .await
            .unwrap_or_default();
        let built = arlm_search::build_search_results(&storage, &results, None).unwrap_or_default();
        let ctx =
            arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
                .unwrap_or_default();
        (ctx, Some(built))
    } else {
        (String::new(), None)
    };

    // Without --llm: return deterministic search results as context
    if !config.llm {
        match config.format {
            Format::FullJson => {
                let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                    "question": config.question,
                    "context": context_str,
                    "project": pname,
                    "llm": false,
                }));
                output.print();
            }
            Format::Jsonl => {
                let pairs: Vec<(String, String)> = query_results
                    .unwrap_or_default()
                    .iter()
                    .map(|r| (r.file_path.clone(), r.content.clone()))
                    .collect();
                let rendered =
                    crate::output::jsonl::render_content_jsonl("question", config.question, &pairs);
                println!("{rendered}");
            }
            Format::Path => {
                output::success(&format!("Context for: {}", config.question));
                println!("\n{context_str}");
            }
            Format::Markdown => {
                println!("## {}\n\n{context_str}", config.question);
            }
            Format::Text => {
                println!("{context_str}");
            }
        }
        return Ok(());
    }

    // With --llm: call LLM with context
    let llm_backend = resolve_backend(config.config, config.backend, config.model)
        .context("failed to create LLM backend")?;

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
            seed: None,
            tools: None,
        })
        .await
        .context("LLM completion failed")?;

    match config.format {
        Format::FullJson => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "question": config.question,
                "answer": response.content,
                "model": response.model,
                "llm": true,
            }));
            output.print();
        }
        Format::Jsonl => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "question": config.question,
                "answer": response.content,
                "model": response.model,
                "llm": true,
            }));
            output.print();
        }
        Format::Path => {
            output::success(&format!("Answer for: {}", config.question));
            println!("\n{}", response.content);
        }
        Format::Markdown => {
            println!("## {}\n\n{}", config.question, response.content);
        }
        Format::Text => {
            println!("{}", response.content);
        }
    }

    Ok(())
}
