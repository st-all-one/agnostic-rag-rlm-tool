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
use tonic::{Request, Response, Status, Streaming};

use arags_proto::proto::index_chunk;

use crate::grpc::error::internal;
use crate::indexing;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{IndexChunk, IndexFile, IndexResponse};

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
    let mut total_chunks: usize = 0;
    let mut distinct_files: usize = 0;
    // Hashes (`chunk_id`, content_hash) of every persisted chunk, kept small for
    // the RLM/exploration staleness hooks below. The full chunk text is NOT
    // retained: each file is chunked, inserted and embedded inline, then its
    // content is dropped, so peak memory stays bounded to a single file. This
    // fixes the all-repo OOM that accumulated every file's bytes in `chunks`
    // for the whole stream (agnostic-rlm-rs-5124).
    let mut persisted_all: Vec<(i64, String)> = Vec::new();
    let mut phase0: Option<(Vec<i64>, usize)> = None;

    while let Some(msg) = stream.message().await.map_err(internal)? {
        match msg.body {
            Some(index_chunk::Body::Init(init)) => {
                project = Some(init.project.clone());
                let pid = store::ensure_project(&state.storage, &init.project, &init.root_path)
                    .map_err(internal)?;
                buffer_id = Some(pid);

                // Phase 0 (stopgap for `agnostic-rlm-rs-20cd`): a re-index must
                // *replace*, not *append*. Delete the buffer's existing chunks
                // (cascade) and purge their vectors once before streaming, so
                // counts stay stable across repeated indexes.
                let storage = state.storage.clone();
                let (existing_ids, deleted_files) =
                    store::blocking(move || store::delete_chunks_for_buffer(&storage, pid))
                        .await
                        .map_err(internal)?;
                if !existing_ids.is_empty() {
                    if let Some(vs) = &state.vector_store {
                        let ids_u64: Vec<u64> =
                            existing_ids.iter().map(|i| *i as u64).collect();
                        if let Err(e) = vs.delete_chunk_ids(&ids_u64).await {
                            tracing::warn!(
                                error = ?e,
                                buffer_id = pid,
                                "failed to purge vectors on re-index; semantic results may be stale until rebuild"
                            );
                        }
                    }
                }
                phase0 = Some((existing_ids, deleted_files));
            }
            Some(index_chunk::Body::File(file)) => {
                let bid = buffer_id
                    .ok_or_else(|| Status::invalid_argument("file message before init"))?;
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
                if chunk_list.is_empty() {
                    continue;
                }
                distinct_files += 1;
                total_chunks += chunk_list.len();

                // Phase 1: persist this file's chunks (transactional, bounded)
                // instead of buffering the whole repo (agnostic-rlm-rs-5124).
                let rel_path = file.rel_path.clone();
                let max_batch = state.config.max_batch_size.max(1);
                let persisted = store::blocking({
                    let storage = state.storage.clone();
                    move || {
                        let flat: Vec<(&str, &indexing::IndexedChunk)> = chunk_list
                            .iter()
                            .map(|c| (rel_path.as_str(), c))
                            .collect();
                        store::insert_chunks_batched(&storage, bid, &flat, max_batch)
                    }
                })
                .await
                .map_err(internal)?;
                persisted_all.extend(persisted.iter().cloned());

                // Phase 2: embed + persist vectors, bounded to this file.
                if let Some(vector_store) = &state.vector_store {
                    let embed_batch = state.config.embedder.batch_size.max(1);
                    let embedder = state.embedder.clone();
                    let buffer_id_u = u64::try_from(bid).unwrap_or(u64::MAX);
                    for batch in persisted.chunks(embed_batch) {
                        let owned_batch: Vec<(i64, String)> = batch.to_vec();
                        let emb = embedder.clone();
                        let bid_u = buffer_id_u;
                        let out = tokio::task::spawn_blocking(move || {
                            let t0 = Instant::now();
                            let batch_len = owned_batch.len();
                            let texts: Vec<&str> =
                                owned_batch.iter().map(|(_, t)| t.as_str()).collect();
                            let res = emb.embed_batch(&texts).map(|vectors| {
                                owned_batch
                                    .into_iter()
                                    .zip(vectors)
                                    .map(|((cid, _), v)| VectorEntry {
                                        chunk_id: u64::try_from(cid).unwrap_or(u64::MAX),
                                        buffer_id: bid_u,
                                        vector: v,
                                    })
                                    .collect::<Vec<_>>()
                            });
                            let elapsed_ms = t0.elapsed().as_millis();
                            match &res {
                                Ok(_) => {
                                    tracing::debug!(batch_len, elapsed_ms, "embedded index batch");
                                }
                                Err(e) => tracing::warn!(error = %e, "batch embedding failed"),
                            }
                            res
                        })
                        .await
                        .map_err(|e| Status::internal(format!("embedding task failed: {e}")))?;
                        match out {
                            Ok(entries) => {
                                if let Err(e) = vector_store.insert_vectors(&entries).await {
                                    tracing::error!(error = %e, "failed to persist vectors, indexing continues");
                                }
                            }
                            Err(e) => tracing::warn!(error = %e, "batch embedding failed"),
                        }
                    }
                }
            }
            None => {}
        }
    }

    let project = project
        .ok_or_else(|| Status::invalid_argument("index stream did not send an init message"))?;
    let buffer_id =
        buffer_id.ok_or_else(|| Status::invalid_argument("index stream missing init"))?;
    let (existing_ids, deleted_files) = phase0.unwrap_or_default();
    let removed = existing_ids.len();
    let net_chunks = (total_chunks as i64).saturating_sub(removed as i64);
    let net_files = (distinct_files as i64).saturating_sub(deleted_files as i64);
    tracing::info!(
        project = %project,
        buffer_id,
        old_chunks = removed,
        net_chunks,
        elapsed_ms = start.elapsed().as_millis(),
        "purged buffer before re-index (stopgap agnostic-rlm-rs-20cd)"
    );

    // Phase 3: bump aggregate counts by this stream's *net* contribution so a
    // re-index (which deleted `old_chunks` in Phase 0) keeps totals stable
    // instead of double-counting.
    let storage = state.storage.clone();
    let embedding_model = state.embedder.name().to_string();
    let embedding_dims = state.embedder.dimensions() as i64;
    store::blocking(move || {
        store::increment_buffer_counts(
            &storage,
            buffer_id,
            net_chunks,
            net_files,
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

    // Phase 4.5 (Explorations, plan 022): bump the project epoch and mark
    // maps stale whose cited anchors no longer match the current chunk hashes.
    if state.config.exploration.enabled {
        let storage = state.storage.clone();
        let project_for_explored = project.clone();
        match store::blocking(move || -> anyhow::Result<(i64, usize)> {
            let epoch = storage.bump_project_epoch(&project_for_explored)?;
            let n = storage.mark_stale_if_anchors_changed(&project_for_explored)?;
            Ok((epoch, n))
        })
        .await
        {
            Ok((epoch, stale)) => {
                if stale > 0 {
                    tracing::info!(project = %project, epoch, stale_maps = stale, "exploration staleness hook");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "exploration staleness hook failed; indexing continues");
            }
        }
    }

    // Phase 4.6 (RLM): mark summaries stale whose source chunk hashes changed
    // in this run — the same hash-driven staleness the QA cache applies. Stale
    // nodes stop surfacing in summary search until volunteers reprocess them.
    if state.config.rlm.enabled {
        let changed: Vec<String> = persisted_all.iter().map(|(_, h)| h.clone()).collect();
        let storage = state.storage.clone();
        match store::blocking(move || storage.mark_rlm_stale_by_hashes(buffer_id, &changed)).await {
            Ok(affected) if !affected.is_empty() => {
                tracing::info!(project = %project, stale_nodes = affected.len(), "rlm staleness hook");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "rlm staleness hook failed; indexing continues"),
        }
    }

    // Phase 5 (RLM): enqueue L1 summary work for the files touched by this
    // stream. Cancellations for claimed jobs ride on the generation bump.
    if state.config.rlm.enabled {
        let chunk_ids: Vec<i64> = persisted_all
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
