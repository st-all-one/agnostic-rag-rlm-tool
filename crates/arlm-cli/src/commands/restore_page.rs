use std::path::Path;

use anyhow::{Context, Result};

use crate::output::Format;
use crate::util::data_dir;

pub fn execute(page_name: &str, project: &Path, format: Format) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let pname = crate::util::project_name(project);

    // Get buffer for project
    let buffer = storage
        .get_buffer_by_name(&pname)
        .context("failed to get buffer")?
        .context("project not found, run 'arlm index' first")?;

    // Search for the page in persisted results
    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
    let results = hybrid
        .search_fts(page_name, buffer.id, 10, None)
        .unwrap_or_default();

    if results.is_empty() {
        crate::output::info(&format!("No page found for '{page_name}'"));
        return Ok(());
    }

    let context_str =
        arlm_search::build_context(&storage, &results, arlm_search::OutputFormat::Prompt, None)
            .unwrap_or_default();

    match format {
        Format::FullJson | Format::Jsonl => {
            let output = crate::output::json::JsonOutput::ok().with_data(serde_json::json!({
                "page": page_name,
                "content": context_str,
                "results": results.len(),
            }));
            output.print();
        }
        Format::Markdown => {
            println!("## {page_name}\n\n{context_str}");
        }
        _ => {
            crate::output::success(&format!("Restored page: {page_name}"));
            println!("\n{context_str}");
        }
    }

    Ok(())
}
