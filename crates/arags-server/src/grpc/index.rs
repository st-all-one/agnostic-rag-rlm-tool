//! Indexing RPC: `IndexProject` (client-streaming).
//!
//! The client discovers and reads files from its OWN filesystem, then streams
//! each file's content here. This handler never touches the client's
//! filesystem — it only receives bytes over gRPC, chunks them deterministically,
//! hashes, extracts entities and persists to SQLite + (optionally) LanceDB.
//! Removing server-side path knowledge closes the arbitrary-file-read footgun
//! described in the security review.

use std::path::Path;
use std::time::Instant;

use arags_storage::VectorEntry;
use futures::stream::{self, StreamExt};
use tonic::{Request, Response, Status, Streaming};

use arags_proto::proto::index_chunk;

use crate::grpc::error::internal;
use crate::indexing;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{IndexChunk, IndexFile, IndexResponse};

/// Default number of concurrent embedding batches when `ARAGS_INDEX_CONCURRENCY`
/// is unset.
const DEFAULT_INDEX_CONCURRENCY: usize = 4;

/// Rough tokens-per-line heuristic used to translate the `[embedder]`
/// `max_tokens`/`overlap_tokens` token budget into a line-based chunk budget
/// for the deterministic line chunker.
const TOKENS_PER_LINE: usize = 10;

/// Map a token budget to a line count, never dropping below one line.
#[must_use]
fn tokens_to_lines(tokens: usize) -> usize {
    if tokens == 0 {
        crate::indexing::DEFAULT_MAX_LINES
    } else {
        (tokens / TOKENS_PER_LINE).max(1)
    }
}

/// Decode a streamed file's content, transparently decompressing if the client
/// sent it zstd-compressed.
fn decode_content(file: &IndexFile) -> Result<String, Status> {
    let bytes = if file.compressed {
        zstd::stream::decode_all(&mut &file.content[..]).map_err(internal)?
    } else {
        file.content.clone()
    };
    String::from_utf8(bytes).map_err(internal)
}

/// Index a project from a client stream of file bytes.
///
/// # Errors
///
/// Returns an error if the stream is malformed, the project is unknown, or any
/// persistence step fails.
pub(crate) async fn handle_index_project(
    state: &AppState,
    request: Request<Streaming<IndexChunk>>,
) -> Result<Response<IndexResponse>, Status> {
    let start = Instant::now();
    let mut stream = request.into_inner();

    let mut project: Option<String> = None;
    let mut buffer_id: Option<i64> = None;
    let mut chunks: Vec<(String, Vec<indexing::IndexedChunk>)> = Vec::new();
    let mut distinct_files: usize = 0;

    while let Some(msg) = stream.message().await.map_err(internal)? {
        match msg.body {
            Some(index_chunk::Body::Init(init)) => {
                project = Some(init.project.clone());
                let pid = store::ensure_project(&state.storage, &init.project, &init.root_path)
                    .map_err(internal)?;
                buffer_id = Some(pid);
            }
            Some(index_chunk::Body::File(file)) => {
                let content = decode_content(&file)?;
                // The server owns chunking (plan 020, D2): derive a line budget
                // from the `[embedder]` token budget so the config is not dead.
                let max_lines = tokens_to_lines(state.config.embedder.max_tokens);
                let overlap = tokens_to_lines(state.config.embedder.overlap_tokens);
                let chunk_list = indexing::index_file_with(
                    Path::new(&file.rel_path),
                    &content,
                    max_lines,
                    overlap,
                );
                distinct_files += 1;
                chunks.push((file.rel_path.clone(), chunk_list));
            }
            None => {}
        }
    }

    let project = project
        .ok_or_else(|| Status::invalid_argument("index stream did not send an init message"))?;
    let buffer_id =
        buffer_id.ok_or_else(|| Status::invalid_argument("index stream missing init"))?;

    let total_chunks: usize = chunks.iter().map(|(_, cs)| cs.len()).sum();

    // Phase 1: persist chunks + texts + FTS + entities in transactional
    // batches of `max_batch_size` (plan 020).
    let storage = state.storage.clone();
    let max_batch = state.config.max_batch_size.max(1);
    let persisted: Vec<(i64, String)> = store::blocking(move || {
        let flat: Vec<(&str, &indexing::IndexedChunk)> = chunks
            .iter()
            .flat_map(|(file, cs)| cs.iter().map(move |c| (file.as_str(), c)))
            .collect();
        store::insert_chunks_batched(&storage, buffer_id, &flat, max_batch)
    })
    .await
    .map_err(internal)?;

    // Phase 2: persist vectors to LanceDB when available.
    if let Some(vector_store) = &state.vector_store {
        // Batch size comes from `server.toml [embedder].batch_size` (plan 020);
        // concurrency stays env-tunable so Docker images can be dialed to match
        // OLLAMA_NUM_PARALLEL without a rebuild (see OLLAMA_EMBED_PROPOSED.md).
        let embed_batch = state.config.embedder.batch_size.max(1);
        let concurrency = std::env::var("ARAGS_INDEX_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(DEFAULT_INDEX_CONCURRENCY)
            .max(1);

        let embedder = state.embedder.clone();
        let buffer_id_u = u64::try_from(buffer_id).unwrap_or(u64::MAX);

        // Split the persisted chunks into batches and embed each batch
        // concurrently. Candle inference on CPU is synchronous, so each
        // batch runs inside `spawn_blocking`; `buffer_unordered` bounds the
        // number of in-flight blocking tasks to `concurrency`.
        let batches: Vec<Vec<(i64, String)>> =
            persisted.chunks(embed_batch).map(|c| c.to_vec()).collect();

        let results = stream::iter(batches)
            .map(|batch| {
                let emb = embedder.clone();
                tokio::task::spawn_blocking(move || {
                    let texts: Vec<&str> = batch.iter().map(|(_, c)| c.as_str()).collect();
                    emb.embed_batch(&texts).map(|vectors| {
                        // `embed_batch` preserves input order, so zipping is safe.
                        batch
                            .into_iter()
                            .zip(vectors)
                            .map(|((cid, _), v)| VectorEntry {
                                chunk_id: u64::try_from(cid).unwrap_or(u64::MAX),
                                buffer_id: buffer_id_u,
                                vector: v,
                            })
                            .collect::<Vec<_>>()
                    })
                })
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;

        let mut entries: Vec<VectorEntry> = Vec::with_capacity(persisted.len());
        for r in results {
            match r {
                Ok(Ok(mut ves)) => entries.append(&mut ves),
                Ok(Err(e)) => tracing::warn!(error = %e, "batch embedding failed"),
                Err(e) => tracing::warn!(error = %e, "embedding task panicked"),
            }
        }

        if let Err(e) = vector_store.insert_vectors(&entries).await {
            tracing::error!(error = %e, "failed to persist vectors, indexing continues");
        }
    }

    // Phase 3: bump aggregate counts by this stream's contribution.
    let storage = state.storage.clone();
    let embedding_model = state.embedder.name().to_string();
    let embedding_dims = state.embedder.dimensions() as i64;
    store::blocking(move || {
        store::increment_buffer_counts(
            &storage,
            buffer_id,
            i64::try_from(total_chunks).unwrap_or(i64::MAX),
            i64::try_from(distinct_files).unwrap_or(i64::MAX),
            &embedding_model,
            embedding_dims,
        )
    })
    .await
    .map_err(internal)?;

    // Phase 4: mark cached answers stale whose source chunks changed/vanished.
    let storage = state.storage.clone();
    if let Ok(n) =
        store::blocking(move || storage.invalidate_stale_cache_for_buffer(buffer_id)).await
    {
        if n > 0 {
            tracing::info!(project = %project, stale_invalidated = n, "qa_cache staleness hook");
        }
    }

    // Phase 5 (RLM): enqueue L1 summary work for the files touched by this
    // stream. Cancellations for claimed jobs ride on the generation bump.
    if state.config.rlm.enabled {
        let chunk_ids: Vec<i64> = persisted
            .iter()
            .filter_map(|(id, _)| i64::try_from(*id).ok())
            .collect();
        let storage = state.storage.clone();
        let project_for_rlm = project.clone();
        match store::blocking(move || -> anyhow::Result<(usize, usize)> {
            let mut files = store::chunks::chunk_file_paths(&storage, &chunk_ids)?;
            files.sort();
            files.dedup();
            if files.is_empty() {
                return Ok((0, 0));
            }
            store::rlm::enqueue_rlm_l1_work(&storage, buffer_id, &project_for_rlm, &files)
        })
        .await
        {
            Ok((new_jobs, reset_jobs)) => {
                if new_jobs + reset_jobs > 0 {
                    tracing::info!(project = %project, new_jobs, reset_jobs, "rlm enqueue hook");
                }
            }
            Err(e) => tracing::warn!(error = %e, "rlm enqueue failed; indexing continues"),
        }
    }

    tracing::info!(
        project = %project,
        files_indexed = distinct_files,
        chunks_created = total_chunks,
        elapsed_ms = start.elapsed().as_millis(),
        "project indexed"
    );

    Ok(Response::new(IndexResponse {
        run_id: uuid::Uuid::now_v7().to_string(),
        files_indexed: i64::try_from(distinct_files).unwrap_or(i64::MAX),
        chunks_created: i64::try_from(total_chunks).unwrap_or(i64::MAX),
        summaries_generated: 0,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
    }))
}
