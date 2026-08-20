use std::path::Path;

use anyhow::{Context, Result};

use crate::output::Format;
use crate::util::data_dir;

pub fn execute(query: &str, project: &Path, format: Format) -> Result<()> {
    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let pname = crate::util::project_name(project);

    let buffer = storage
        .get_buffer_by_name(&pname)
        .context("failed to get buffer")?
        .context("project not found, run 'arlm index' first")?;

    let bm25 = arlm_search::Bm25Search::new(&storage).context("failed to create BM25 search")?;
    let hybrid = arlm_search::HybridSearch::new(bm25, None, None);
    let results = hybrid
        .search_fts(query, buffer.id, 10, None)
        .unwrap_or_default();

    match format {
        Format::Json => {
            let entities: Vec<_> = results
                .iter()
                .filter_map(|r| {
                    storage
                        .get_chunk_entities(r.chunk_id)
                        .ok()
                        .flatten()
                        .map(|e| {
                            serde_json::json!({
                                "chunk_id": r.chunk_id,
                                "entities": e,
                            })
                        })
                })
                .collect();
            let output = crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({ "entities": entities }));
            output.print();
        }
        _ => {
            if results.is_empty() {
                crate::output::info(&format!("No entities found for '{query}'"));
            } else {
                for r in &results {
                    if let Ok(Some(entities)) = storage.get_chunk_entities(r.chunk_id) {
                        println!("Chunk {}: {}", r.chunk_id, entities.join(", "));
                    }
                }
            }
        }
    }

    Ok(())
}
