use std::path::Path;

use anyhow::{Context, Result};
use tracing::warn;

use crate::config::Config;
use crate::embedding::{build_embedder_from_config, open_vector_store, vector_dir};
use crate::output::{self, Format};
use crate::util::{data_dir, project_name};

use arlm_storage::VectorEntry;
use arlm_embedding::embedder::Embedder;
use std::sync::Arc;

/// Number of chunks embedded per Ollama batch request during indexing.
const EMBED_BATCH_SIZE: usize = 64;

pub struct IndexConfig<'a> {
    pub path: &'a Path,
    pub chunk_size: usize,
    pub ignore_patterns: &'a [String],
    pub force_include: &'a [String],
    pub watch: bool,
    pub project: &'a Path,
    pub format: Format,
    pub verbose: bool,
    pub config: &'a Config,
}

#[allow(clippy::needless_pass_by_value)]
pub async fn execute(config: IndexConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_index");

    let absolute_path = config
        .path
        .canonicalize()
        .with_context(|| format!("failed to resolve path: {}", config.path.display()))?;

    let pname = project_name(config.project);

    if config.verbose {
        output::info(&format!("Indexing {}...", absolute_path.display()));
    }

    let storage = arlm_storage::Storage::open(&data_dir()).context("failed to open storage")?;
    storage.ensure_uuids().ok();

    let buffer = storage
        .get_buffer_by_name(&pname)
        .context("failed to check buffer")?;

    let _buffer_id = if let Some(buf) = buffer {
        buf.id
    } else {
        storage
            .insert_buffer(&arlm_storage::sqlite::buffers::NewBuffer {
                name: pname.clone(),
                path: absolute_path.to_string_lossy().to_string(),
            })
            .context("failed to create buffer")?
    };

    let knowledge = arlm_memory::KnowledgeEngine::new(storage.clone());
    let opts = arlm_memory::knowledge::IndexOptions {
        max_chunk_bytes: config.chunk_size * 4,
        ignore_patterns: config.ignore_patterns.to_vec(),
        force_include: config.force_include.to_vec(),
        ..Default::default()
    };

    let progress = indicatif::ProgressBar::new_spinner();
    progress.set_style(
        indicatif::ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg} [{elapsed_precise}]")
            .map_err(|e| anyhow::anyhow!("invalid template: {e}"))?,
    );
    progress.set_message("Indexing files...");

    let result = knowledge
        .index_directory(&pname, &absolute_path, &opts)
        .context("failed to index directory")?;

    // Auto-populate FTS5 index after indexing
    let bm25 = arlm_search::Bm25Search::new(knowledge.storage())
        .context("failed to create BM25 search for FTS population")?;
    let _fts_count = bm25
        .populate_fts()
        .context("failed to populate FTS index")?;

    // Populate the semantic (BGE-M3) vector store. Previously this step was
    // missing entirely (the knowledge engine explicitly "does NOT compute
    // embeddings"), so semantic search had no vectors to retrieve. We rebuild
    // this buffer's vector store from scratch and embed every chunk.
    if let Some(buffer) = storage
        .get_buffer_by_name(&pname)
        .context("failed to check buffer")?
    {
        embed_buffer(config.config, &storage, buffer.id).await;
    }

    progress.finish_and_clear();

    match config.format {
        Format::FullJson | Format::Jsonl => {
            let dur: u64 = result.duration_ms.try_into().unwrap_or(u64::MAX);
            let output = crate::output::json::JsonOutput::ok()
                .with_data(serde_json::json!({
                    "project": pname,
                    "path": absolute_path.display().to_string(),
                    "files_processed": result.files_processed,
                    "chunks_created": result.chunks_created,
                    "duration_ms": dur,
                }))
                .with_metadata("duration_ms", dur);
            output.print();
        }
        Format::Path => {
            let dur_ms = u64::try_from(result.duration_ms).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)]
            let dur_secs = dur_ms as f64 / 1000.0;
            output::success(&format!(
                "Indexed {} files → {} chunks in {dur_secs:.1}s",
                result.files_processed, result.chunks_created,
            ));
            output::info(&format!("Database: {}/knowledge.db", data_dir().display()));
        }
        Format::Markdown => {
            let dur_ms = u64::try_from(result.duration_ms).unwrap_or(u64::MAX);
            #[allow(clippy::cast_precision_loss)]
            let dur_secs = dur_ms as f64 / 1000.0;
            println!(
                "# Indexing Complete\n\n- **Files:** {}\n- **Chunks:** {}\n- **Duration:** {dur_secs:.1}s\n",
                result.files_processed, result.chunks_created,
            );
        }
        Format::Text => {
            println!(
                "Indexed {} files into {} chunks. Project: {}.",
                result.files_processed, result.chunks_created, pname,
            );
        }
    }

    // Watch mode: monitor for changes and reindex
    if config.watch {
        use arlm_memory::watch::{WatchMonitor, WatchOptions};

        output::info("Watching for file changes... (Ctrl+C to stop)");

        let watch_opts = WatchOptions {
            debounce_ms: 500,
            recursive: true,
        };

        let handle = WatchMonitor::watch(&absolute_path, &watch_opts)
            .context("failed to start file watcher")?;

        loop {
            match handle.recv() {
                Ok(event) => {
                    let is_relevant = event.paths.iter().any(|p| {
                        p.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
                            matches!(
                                ext,
                                "rs" | "py"
                                    | "js"
                                    | "ts"
                                    | "go"
                                    | "java"
                                    | "c"
                                    | "cpp"
                                    | "h"
                                    | "rb"
                                    | "md"
                                    | "txt"
                                    | "json"
                                    | "yaml"
                                    | "toml"
                            )
                        })
                    });

                    if !is_relevant {
                        continue;
                    }

                    output::info(&format!(
                        "Detected {} change(s), reindexing...",
                        event.paths.len()
                    ));

                    let storage = arlm_storage::Storage::open(&data_dir())
                        .context("failed to open storage")?;

                    let knowledge = arlm_memory::KnowledgeEngine::new(storage.clone());
                    let opts = arlm_memory::knowledge::IndexOptions {
                        max_chunk_bytes: config.chunk_size * 4,
                        ignore_patterns: config.ignore_patterns.to_vec(),
                        ..Default::default()
                    };

                    match knowledge.index_directory(&pname, &absolute_path, &opts) {
                        Ok(result) => {
                            // Re-populate FTS after reindex
                            if let Ok(bm25) = arlm_search::Bm25Search::new(knowledge.storage()) {
                                let _ = bm25.populate_fts();
                            }
                            // Re-embed into the semantic vector store.
                            if let Some(buf) = knowledge
                                .storage()
                                .get_buffer_by_name(&pname)
                                .ok()
                                .flatten()
                            {
                                embed_buffer(config.config, knowledge.storage(), buf.id).await;
                            }
                            output::success(&format!(
                                "Reindexed {} files → {} chunks",
                                result.files_processed, result.chunks_created,
                            ));
                        }
                        Err(e) => {
                            output::error(&format!("Reindex failed: {e}"));
                        }
                    }
                }
                Err(e) => {
                    output::error(&format!("Watch error: {e}"));
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Embed every chunk of `buffer_id` into the per-buffer vector store, replacing
/// any previous vectors for that buffer.
///
/// This is the missing piece that makes semantic (BGE-M3) filtering functional:
/// Embed a batch of `(chunk_id, text)` pairs and push the resulting
/// `VectorEntry`s. Uses a single batched request; on failure it falls back to
/// per-chunk embedding so one bad chunk never drops the whole batch.
fn embed_pending(
    entries: &mut Vec<VectorEntry>,
    embedder: &Arc<dyn Embedder>,
    pending: &[(u64, String)],
    buffer_u: u64,
) {
    let texts: Vec<&str> = pending.iter().map(|(_, t)| t.as_str()).collect();
    match embedder.embed_batch(&texts) {
        Ok(vectors) => {
            for (meta, vector) in pending.iter().zip(vectors) {
                entries.push(VectorEntry {
                    chunk_id: meta.0,
                    buffer_id: buffer_u,
                    vector,
                });
            }
        }
        Err(e) => {
            warn!(error = %e, "batch embedding failed, falling back to per-chunk");
            for (chunk_u, text) in pending {
                match embedder.embed(text) {
                    Ok(vector) => entries.push(VectorEntry {
                        chunk_id: *chunk_u,
                        buffer_id: buffer_u,
                        vector,
                    }),
                    Err(e2) => warn!(chunk_id = chunk_u, error = %e2, "chunk embedding failed"),
                }
            }
        }
    }
}

/// the knowledge engine stores chunk text but never computes embeddings, so the
/// vector store was always empty. We rebuild this buffer's store from scratch
/// to avoid stale vectors from deleted chunks.
async fn embed_buffer(config: &Config, storage: &arlm_storage::Storage, buffer_id: i64) {
    let embedder = build_embedder_from_config(config, "search_document: ");

    // Rebuild from scratch: remove the old vector store directory for this buffer.
    let vdir = vector_dir(buffer_id);
    if vdir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&vdir) {
            warn!(error = %e, "failed to clear old vector store, reusing existing");
        }
    }

    let vstore = match open_vector_store(buffer_id, embedder.dimensions()).await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to open vector store, skipping embeddings");
            return;
        }
    };

    let chunks = match storage.list_chunks(buffer_id) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "failed to list chunks for embedding");
            return;
        }
    };

    let Ok(buffer_u) = u64::try_from(buffer_id) else {
        warn!(buffer_id, "negative buffer id, skipping embeddings");
        return;
    };

    let mut entries: Vec<VectorEntry> = Vec::with_capacity(chunks.len());
    let mut pending: Vec<(u64, String)> = Vec::with_capacity(EMBED_BATCH_SIZE);
    for c in &chunks {
        let Ok(chunk_u) = u64::try_from(c.id) else {
            warn!(chunk_id = c.id, "negative chunk id, skipping chunk");
            continue;
        };
        match storage.get_chunk_content(c.id) {
            Ok(Some(text)) => pending.push((chunk_u, text)),
            Ok(None) => {}
            Err(e) => warn!(chunk_id = c.id, error = %e, "chunk content read failed"),
        }
        if pending.len() >= EMBED_BATCH_SIZE {
            embed_pending(&mut entries, &embedder, &pending, buffer_u);
            pending.clear();
        }
    }
    if !pending.is_empty() {
        embed_pending(&mut entries, &embedder, &pending, buffer_u);
    }

    if entries.is_empty() {
        return;
    }

    match vstore.insert_vectors(&entries).await {
        Ok(()) => tracing::info!(count = entries.len(), "embedded chunks into vector store"),
        Err(e) => warn!(error = %e, "failed to insert vectors"),
    }
}
