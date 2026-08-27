//! Indexing RPC: `IndexProject` (client-streaming).
//!
//! The client discovers and reads files from its OWN filesystem, then streams
//! each file's content here. This handler never touches the client's
//! filesystem — it only receives bytes over gRPC, chunks them deterministically,
//! hashes, extracts entities and persists to SQLite + (optionally) LanceDB.
//! Removing server-side path knowledge closes the arbitrary-file-read footgun
//! described in the security review.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use arags_embedding::embedder::EmbeddingResult;
use arags_storage::VectorEntry;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tonic::{Request, Response, Status, Streaming};
use tracing::{debug, info, instrument, warn};

use arags_proto::proto::index_chunk;

use crate::grpc::error::internal;
use crate::indexing;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::{IndexChunk, IndexFile, IndexResponse};

/// Spawned embed task: returns the vector entries to persist, or the embedder
/// error. Type alias keeps the `JoinHandle` spelling uniform across the loop.
type EmbedHandle = JoinHandle<EmbeddingResult<Vec<VectorEntry>>>;

/// Drop guard that aborts every spawned embed task.
///
/// Guarantees no blocking embed task outlives [`index_stream_loop`] (issue
/// `agnostic-rlm-rs-e5d0`): if the client disconnects or any step errors, all
/// pending embeds are cancelled before the handler returns, so the CPU stops
/// promptly and the pooled DB connections are released for the next request.
struct EmbedAbortGuard(Vec<(EmbedHandle, Vec<i64>)>);

impl EmbedAbortGuard {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn push(&mut self, handle: EmbedHandle, chunk_ids: Vec<i64>) {
        self.0.push((handle, chunk_ids));
    }

    /// Pop the most-recently-spawned pending embed task, if any, along with the
    /// chunk ids it was meant to vectorize.
    fn pop(&mut self) -> Option<(EmbedHandle, Vec<i64>)> {
        self.0.pop()
    }

    /// Abort every still-pending embed task and forget the handles.
    fn abort_all(&mut self) {
        for (handle, _) in &self.0 {
            handle.abort();
        }
        self.0.clear();
    }
}

impl Drop for EmbedAbortGuard {
    fn drop(&mut self) {
        self.abort_all();
    }
}

/// Rough tokens-per-line heuristic used to translate the `[embedder]`
/// `max_tokens`/`overlap_tokens` token budget into a line-based chunk budget
/// for the deterministic line chunker.
const TOKENS_PER_LINE: usize = 10;

/// Map a token budget to a line count, never dropping below one line.
#[must_use]
pub(crate) fn tokens_to_lines(tokens: usize) -> usize {
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
/// This is the thin gRPC entry point. It delegates the stream-processing loop
/// to [`index_stream_loop`], which is generic over the stream source so it can
/// be exercised directly in tests (e.g. a `ReceiverStream`).
///
/// # Errors
///
/// Returns an error if the stream is malformed, the project is unknown, or any
/// persistence step fails.
pub(crate) async fn handle_index_project(
    state: &AppState,
    request: Request<Streaming<IndexChunk>>,
    created_by: Option<String>,
) -> Result<Response<IndexResponse>, Status> {
    index_stream_loop(state, request.into_inner(), created_by).await
}

/// Core indexing loop, generic over the stream so it is unit-testable.
///
/// The loop reads [`IndexChunk`] messages until the client closes the stream
/// (`None`) or the stream errors (`Err`). On *either* condition it returns
/// promptly, dropping `AbortOnDrop`-guarded embed tasks and releasing every
/// pooled SQLite connection/transaction (none are held across iterations).
///
/// Per issue `agnostic-rlm-rs-e5d0`, a client that disconnects after `Init`
/// must NOT leave the buffer purged — the destructive Phase-0 replace is
/// deferred to the first `File` message (see the `phase0_done` guard below),
/// and a mid-stream disconnect aborts pending embeds and frees the pool so a
/// subsequent RLM `claim rlm_job` does not fail until restart.
///
/// # Errors
///
/// Returns an error only for malformed streams or fatal persistence failures.
/// A clean client disconnect returns a successful (possibly empty) response.
#[instrument(
    skip_all,
    fields(buffer_id = tracing::field::Empty, project = tracing::field::Empty)
)]
pub(crate) async fn index_stream_loop<S>(
    state: &AppState,
    mut stream: S,
    created_by: Option<String>,
) -> Result<Response<IndexResponse>, Status>
where
    S: futures::Stream<Item = Result<IndexChunk, Status>> + Unpin + Send,
{
    let start = Instant::now();
    // Authorship: the embedding model (chunks are produced by the embedder, not
    // an LLM) and the authenticated session username threaded from the gRPC
    // layer (issue `agnostic-rlm-rs-786a`). `None` only occurs in hermetic
    // tests that bypass the auth wrapper.
    let model_name = state.embedder.name();
    let model: Option<&str> = Some(model_name);

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
    // Immutable supersede (issue `agnostic-rlm-rs-8dcc`): a re-index no longer
    // *deletes* the buffer's chunks — it snapshots the currently-active chunk
    // keys on the first `File` message, then inserts NEW active rows and
    // *retires* (is_active = 0) the previous version of each matching key as
    // Phase 1 runs. Any active key still unmatched at end-of-stream is an
    // orphan (file removed / chunk moved) and is retired then. `snapshot` maps
    // `(file_path, line_start, line_end) -> chunk_id`; `remaining_active` is the
    // set of snapshot keys not yet superseded, drained as Phase 1 confirms each
    // key.
    let mut snapshot: Option<HashMap<store::chunks::ChunkKey, i64>> = None;
    let mut remaining_active: HashSet<store::chunks::ChunkKey> = HashSet::new();
    // Accounting counters accumulated across the stream (replaces the old
    // delete-based net math): how many new rows were inserted and how many
    // previous versions were retired.
    let mut pre_active: usize = 0;
    let mut total_inserted: usize = 0;
    let mut total_retired: usize = 0;
    // Deferred snapshot (issue `agnostic-rlm-rs-e5d0`): nothing destructive may
    // run until the stream actually delivers a `File`. A client that
    // disconnects right after `Init` (or never sends files) must NOT leave the
    // buffer touched — that broken state is what breaks RLM claims until
    // restart (`agnostic-rlm-rs-ccc3`). So the supersede snapshot is taken only
    // on the first `File` message.
    let mut phase0_done = false;
    // Live embed tasks. The guard aborts every handle still in the vec on drop,
    // so a disconnect/error mid-stream cannot leak a blocking task.
    let mut embed_abort = EmbedAbortGuard::new();

    while let Some(msg) = stream.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                // Stream error: abort any in-flight embeds and return promptly.
                // No pooled connection is held here (all are scoped inside
                // `store::blocking` closures that already returned).
                warn!(error = %e, "index stream errored; aborting cleanly");
                embed_abort.abort_all();
                return Err(e);
            }
        };

        match msg.body {
            Some(index_chunk::Body::Init(init)) => {
                let pid = store::ensure_project(&state.storage, &init.project, &init.root_path)
                    .map_err(internal)?;
                project = Some(init.project.clone());
                buffer_id = Some(pid);
                tracing::Span::current().record("buffer_id", pid);
                tracing::Span::current().record("project", &init.project);
                debug!(phase = "init", project = %init.project, buffer_id = pid, "index init received");
            }
            Some(index_chunk::Body::File(file)) => {
                let bid = buffer_id
                    .ok_or_else(|| Status::invalid_argument("file message before init"))?;

                // Phase 0 (issue `agnostic-rlm-rs-8dcc`): supersede instead of
                // delete. Snapshot the currently-active chunk keys so Phase 1
                // can retire the previous version of each as it re-inserts, and
                // so the end-of-stream orphan pass knows what was removed. This
                // is still *deferred* to the first `File` (e5d0): a client that
                // disconnects after `Init` leaves the buffer untouched.
                if !phase0_done {
                    let t0 = Instant::now();
                    let storage = state.storage.clone();
                    let snap = store::blocking({
                        let storage = storage.clone();
                        move || store::chunks::snapshot_active_chunks(&storage, bid)
                    })
                    .await
                    .map_err(internal)?;
                    pre_active = snap.len();
                    remaining_active = snap.keys().cloned().collect();
                    debug!(
                        phase = "phase0_supersede",
                        elapsed_ms = t0.elapsed().as_millis() as u64,
                        buffer_id = bid,
                        active = pre_active,
                        "phase0 snapshotted active chunks for supersede"
                    );
                    snapshot = Some(snap);
                    phase0_done = true;
                }

                let t1 = Instant::now();
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
                // `store::blocking` acquires a pooled connection, runs the
                // transaction and drops it before returning — nothing is held
                // across loop iterations.
                let rel_path = file.rel_path.clone();
                let max_batch = state.config.max_batch_size.max(1);
                let snap = snapshot
                    .clone()
                    .ok_or_else(|| internal("index snapshot missing despite phase0_done"))?;
                let res = store::blocking({
                    let storage = state.storage.clone();
                    let snap = snap.clone();
                    let cb = created_by.clone();
                    move || {
                        let flat: Vec<(&str, &indexing::IndexedChunk)> =
                            chunk_list.iter().map(|c| (rel_path.as_str(), c)).collect();
                        store::insert_chunks_batched(
                            &storage,
                            bid,
                            &flat,
                            max_batch,
                            &snap,
                            cb.as_deref(),
                            model,
                        )
                    }
                })
                .await
                .map_err(internal)?;
                debug!(
                    phase = "phase1_persist",
                    elapsed_ms = t1.elapsed().as_millis() as u64,
                    buffer_id = bid,
                    chunk_count = res.persisted.len(),
                    created_by = created_by.as_deref(),
                    model,
                    "phase1 persisted file chunks"
                );
                persisted_all.extend(res.persisted.iter().cloned());
                for key in &res.handled_keys {
                    remaining_active.remove(key);
                }
                total_inserted += res.inserted;
                total_retired += res.retired_ids.len();

                // Retired (superseded) chunks must vanish from semantic search
                // immediately: purge their usearch vectors (their FTS rows were
                // already dropped by the retire helper inside the transaction).
                if !res.retired_ids.is_empty() {
                    if let Some(vs) = &state.vector_store {
                        let ids_u64: Vec<u64> = res.retired_ids.iter().map(|i| *i as u64).collect();
                        if let Err(e) = vs.delete_chunk_ids(&ids_u64).await {
                            warn!(
                                error = ?e,
                                buffer_id = bid,
                                "failed to purge superseded vectors; semantic results may be stale until rebuild"
                            );
                        }
                    }
                }

                // Phase 2: embed + persist vectors, bounded to this file.
                if let Some(vector_store) = &state.vector_store {
                    let embed_batch = state.config.embedder.batch_size.max(1);
                    let embedder = state.embedder.clone();
                    let buffer_id_u = u64::try_from(bid).unwrap_or(u64::MAX);
                    // Clone the capped index-embed pool + the in-flight counter
                    // so the blocking task can confine candle's matmul and
                    // report backpressure signal (issue `agnostic-rlm-rs-6690`).
                    let pool_threads = state.index_embed_pool.current_num_threads();
                    for batch in res.persisted.chunks(embed_batch) {
                        let owned_batch: Vec<(i64, String)> = batch.to_vec();
                        let batch_chunk_ids: Vec<i64> =
                            owned_batch.iter().map(|(cid, _)| *cid).collect();
                        let emb = embedder.clone();
                        let bid_u = buffer_id_u;
                        let pool = state.index_embed_pool.clone();
                        let active = state.active_index_embeds.clone();
                        let handle = tokio::task::spawn_blocking(move || {
                            let t0 = Instant::now();
                            let batch_len = owned_batch.len();
                            let texts: Vec<&str> =
                                owned_batch.iter().map(|(_, t)| t.as_str()).collect();
                            // Run the embed inside the capped pool so candle's
                            // internal rayon matmul stays off the global pool and
                            // leaves cores free for concurrent query serving.
                            let res = pool
                                .install(|| {
                                    active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                    let r = emb.embed_batch(&texts);
                                    active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                    r
                                })
                                .map(|vectors| {
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
                            debug!(
                                phase = "phase2_embed_batch",
                                elapsed_ms = t0.elapsed().as_millis() as u64,
                                batch_len,
                                pool_threads,
                                "embedded index batch on capped pool"
                            );
                            res
                        });
                        embed_abort.push(handle, batch_chunk_ids);
                    }

                    // Await all batches for this file. On a join failure we bail
                    // out; the still-pending handles remain in `embed_abort` and
                    // are aborted by its `Drop` impl (or the explicit call).
                    while let Some((handle, batch_chunk_ids)) = embed_abort.pop() {
                        let out = handle.await.map_err(|e| {
                            embed_abort.abort_all();
                            Status::internal(format!("embedding task failed: {e}"))
                        })?;
                        match out {
                            Ok(entries) => {
                                if let Err(e) = vector_store.insert_vectors(&entries).await {
                                    warn!(
                                        error = %e,
                                        buffer_id = bid,
                                        n_chunks = batch_chunk_ids.len(),
                                        "failed to persist vectors; marking chunks pending_vector"
                                    );
                                    if let Err(m) = state
                                        .storage
                                        .mark_chunks_pending_vector(bid, &batch_chunk_ids)
                                    {
                                        warn!(error = %m, "failed to mark chunks pending_vector");
                                    } else {
                                        debug!(
                                            buffer_id = bid,
                                            n_marked = batch_chunk_ids.len(),
                                            "marked chunks pending_vector for re-embed"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(
                                    error = %e,
                                    buffer_id = bid,
                                    n_chunks = batch_chunk_ids.len(),
                                    "batch embedding failed; marking chunks pending_vector"
                                );
                                if let Err(m) = state
                                    .storage
                                    .mark_chunks_pending_vector(bid, &batch_chunk_ids)
                                {
                                    warn!(error = %m, "failed to mark chunks pending_vector");
                                } else {
                                    debug!(
                                        buffer_id = bid,
                                        n_marked = batch_chunk_ids.len(),
                                        "marked chunks pending_vector for re-embed"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            None => {}
        }
    }

    // Clean client disconnect (`None`): finish the bookkeeping phases with
    // whatever was persisted. No connection or embed task is held here.
    // Record the disconnect with its elapsed time and the count of embed
    // tasks that the `EmbedAbortGuard` will cancel on drop — this is the
    // diagnostic signal called for by issue `agnostic-rlm-rs-ccc3` to confirm
    // no pooled connection/transaction leaks past the handler.
    let aborted_embed_tasks = embed_abort.0.len();
    warn!(
        reason = "client_disconnect",
        elapsed_ms = start.elapsed().as_millis() as u64,
        aborted_embed_tasks,
        "index stream ended; pooled connections/tx released"
    );

    // End-of-stream orphan pass (issue `agnostic-rlm-rs-8dcc`): any active chunk
    // key snapshotted at Phase 0 that Phase 1 never re-inserted is now orphaned
    // (file removed or chunk moved). Retire it softly so search never surfaces
    // it, and purge its vector. Scoped inside `store::blocking` so no pooled
    // connection is held across the await.
    if let Some(snap) = &snapshot {
        if !remaining_active.is_empty() {
            let orphan_ids: Vec<i64> = remaining_active.iter().map(|k| snap[k]).collect();
            let storage = state.storage.clone();
            store::blocking({
                let storage = storage.clone();
                let ids = orphan_ids.clone();
                move || -> anyhow::Result<()> {
                    for id in &ids {
                        store::chunks::retire_chunk(&storage, *id, None)?;
                    }
                    Ok(())
                }
            })
            .await
            .map_err(internal)?;
            if let Some(vs) = &state.vector_store {
                let ids_u64: Vec<u64> = orphan_ids.iter().map(|i| *i as u64).collect();
                if let Err(e) = vs.delete_chunk_ids(&ids_u64).await {
                    warn!(
                        error = ?e,
                        orphan_count = orphan_ids.len(),
                        "failed to purge orphan vectors; semantic results may be stale until rebuild"
                    );
                }
            }
            info!(
                buffer_id = buffer_id,
                orphan_count = orphan_ids.len(),
                "retired orphaned chunks (removed files / moved chunks)"
            );
        }
    }

    let project = project
        .ok_or_else(|| Status::invalid_argument("index stream did not send an init message"))?;
    let buffer_id =
        buffer_id.ok_or_else(|| Status::invalid_argument("index stream missing init"))?;
    // Buffer-count delta under supersede: the final active count equals the
    // start-of-stream active count minus orphaned keys plus truly-new rows
    // (`inserted - retired`, since each retired version is matched by one new
    // row). Re-indexing a disjoint file set thus keeps totals stable.
    let orphan_count = remaining_active.len();
    let net_chunks = (total_inserted as i64)
        .saturating_sub(total_retired as i64)
        .saturating_sub(orphan_count as i64);
    let orphan_files: HashSet<&String> = remaining_active.iter().map(|(fp, _, _)| fp).collect();
    let net_files = (distinct_files as i64).saturating_sub(orphan_files.len() as i64);
    info!(
        project = %project,
        buffer_id,
        pre_active,
        net_chunks,
        orphan_count,
        elapsed_ms = start.elapsed().as_millis(),
        "superseded buffer chunks on re-index (agnostic-rlm-rs-8dcc)"
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
            info!(project = %project, stale_invalidated = n, "qa_cache staleness hook");
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
                    info!(project = %project, epoch, stale_maps = stale, "exploration staleness hook");
                }
            }
            Err(e) => {
                warn!(error = %e, "exploration staleness hook failed; indexing continues");
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
                info!(project = %project, stale_nodes = affected.len(), "rlm staleness hook");
            }
            Ok(_) => {}
            Err(e) => warn!(error = %e, "rlm staleness hook failed; indexing continues"),
        }
    }

    // Phase 5 (RLM): enqueue L1 summary work for the files touched by this
    // stream. Cancellations for claimed jobs ride on the generation bump.
    if state.config.rlm.enabled {
        let chunk_ids: Vec<i64> = persisted_all
            .iter()
            .filter_map(|(id, _)| i64::try_from(*id).ok())
            .collect();
        let quorum_n = state.config.quorum.n.max(1);
        let storage = state.storage.clone();
        let project_for_rlm = project.clone();
        match store::blocking(move || -> anyhow::Result<(usize, usize)> {
            let mut files = store::chunks::chunk_file_paths(&storage, &chunk_ids)?;
            files.sort();
            files.dedup();
            if files.is_empty() {
                return Ok((0, 0));
            }
            store::rlm::enqueue_rlm_l1_work(&storage, buffer_id, &project_for_rlm, &files, quorum_n)
        })
        .await
        {
            Ok((new_jobs, reset_jobs)) => {
                if new_jobs + reset_jobs > 0 {
                    info!(project = %project, new_jobs, reset_jobs, "rlm enqueue hook");
                }
            }
            Err(e) => warn!(error = %e, "rlm enqueue failed; indexing continues"),
        }
    }

    info!(
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::Path;
    use std::sync::Arc;

    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use arags_embedding::embedder::Embedder;
    use arags_embedding::embedder::lightweight::LightweightEmbedder;
    use arags_storage::Storage;
    use arags_storage::VectorStore;
    use arags_storage::sqlite::rlm::{DEFAULT_RLM_LEASE_MS, NewRlmJob};

    use crate::config::ServerConfig;
    use crate::grpc::index::index_stream_loop;
    use crate::grpc::index::tokens_to_lines;
    use crate::state::AppState;
    use arags_proto::proto::{IndexChunk, IndexFile, IndexInit, index_chunk};

    /// Build a minimal `AppState` with storage only — no vector store, no
    /// embedder backend that touches the network. Mirrors the `fixture()`
    /// helper in the explorations tests but keeps RLM/exploration hooks off so
    /// the unit tests stay hermetic.
    fn fixture() -> (tempfile::TempDir, Storage, AppState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::open(dir.path()).expect("open storage");
        seed_project(&storage, "proj", 1);
        seed_chunks(
            &storage,
            1,
            &[("src/a.rs", "hash-a"), ("src/b.rs", "hash-b")],
        );

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;

        // No vector store / question / rlm / exploration stores.
        let state = AppState::with_vector_stores(storage.clone(), cfg, None, None, None, None)
            .expect("app state");
        (dir, storage, state)
    }

    fn seed_project(storage: &Storage, name: &str, id: i64) {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO buffers (id, name, path) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![id, name, "/tmp/proj"],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn seed_chunks(storage: &Storage, buffer_id: i64, rows: &[(&str, &str)]) {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                for (path, hash) in rows {
                    c.execute(
                        "DELETE FROM chunks WHERE buffer_id = ?1 AND file_path = ?2",
                        rusqlite::params![buffer_id, path],
                    )?;
                    c.execute(
                        "INSERT INTO chunks \
                         (buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash) \
                         VALUES (?1, ?2, 0, 1, 1, 1, ?3)",
                        rusqlite::params![buffer_id, path, hash.as_bytes()],
                    )?;
                }
                Ok(())
            })
            .unwrap();
    }

    fn chunk_count(storage: &Storage, buffer_id: i64) -> i64 {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM chunks WHERE buffer_id = ?1 AND is_active = 1",
                    rusqlite::params![buffer_id],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    /// Return true if any chunk text for `buffer_id` contains `needle`.
    /// Used to prove a re-index *replaced* (not appended) content: the old
    /// run's marker must vanish from `chunk_texts`/`chunks_fts` after a
    /// subsequent re-index (issue `agnostic-rlm-rs-20cd`). Only active chunks
    /// are considered (history is filtered out), mirroring live search.
    fn chunk_text_has(storage: &Storage, buffer_id: i64, needle: &str) -> bool {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                let mut stmt = c.prepare(
                    "SELECT ct.content FROM chunk_texts ct \
                     JOIN chunks c ON c.id = ct.chunk_id WHERE c.buffer_id = ?1 AND c.is_active = 1",
                )?;
                let mut rows =
                    stmt.query_map(rusqlite::params![buffer_id], |r| r.get::<_, String>(0))?;
                let mut found = false;
                while let Some(Ok(content)) = rows.next() {
                    if content.contains(needle) {
                        found = true;
                        break;
                    }
                }
                Ok(found)
            })
            .unwrap()
    }

    fn init_chunk(project: &str) -> IndexChunk {
        IndexChunk {
            body: Some(index_chunk::Body::Init(IndexInit {
                project: project.to_string(),
                root_path: "/tmp/proj".to_string(),
                force_include: vec![],
                exclude_patterns: vec![],
            })),
        }
    }

    fn file_chunk(rel_path: &str, content: &str) -> IndexChunk {
        IndexChunk {
            body: Some(index_chunk::Body::File(IndexFile {
                rel_path: rel_path.to_string(),
                content: content.as_bytes().to_vec(),
                compressed: false,
                size_bytes: content.len() as i64,
            })),
        }
    }

    #[tokio::test]
    async fn disconnect_after_init_keeps_deferred_delete_pending() {
        let (_dir, storage, state) = fixture();

        // Stream yields exactly one Init then the sender is dropped
        // (simulated client disconnect right after Init).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(4);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        drop(tx);
        let stream = ReceiverStream::new(rx);

        let resp = index_stream_loop(&state, stream, None).await;
        assert!(resp.is_ok(), "handler must return cleanly on disconnect");

        // The deferred Phase-0 delete must NOT have run: the two pre-seeded
        // chunks for buffer 1 are still present.
        assert_eq!(
            chunk_count(&storage, 1),
            2,
            "deferred delete must not run when only Init was received"
        );
    }

    #[tokio::test]
    async fn disconnect_mid_stream_releases_pooled_connection() {
        let (_dir, storage, state) = fixture();

        // Stream yields Init + one File then the sender is dropped (simulated
        // mid-index disconnect). The File triggers the deferred delete + a
        // persist; afterward the pool must be free.
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(4);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk("src/a.rs", "fn main() {}\n")))
            .await
            .unwrap();
        drop(tx);
        let stream = ReceiverStream::new(rx);

        let resp = index_stream_loop(&state, stream, None).await;
        assert!(resp.is_ok(), "handler must return cleanly on disconnect");

        // A subsequent direct Storage operation must succeed — proving no
        // pooled connection / open transaction leaked from the aborted handler
        // (this is what broke RLM `claim rlm_job` until restart, issue
        // agnostic-rlm-rs-ccc3).
        let conn = storage.connection();
        assert!(
            conn.is_ok(),
            "pooled connection must be available after handler abort"
        );
        assert!(
            chunk_count(&storage, 1) >= 0,
            "post-disconnect query must succeed"
        );
    }

    #[tokio::test]
    async fn disconnect_mid_index_keeps_rlm_claim_working() {
        let (_dir, storage, state) = fixture();

        // Seed a PENDING RLM job for the same project ("proj") directly via
        // Storage, mirroring `crates/arags-storage/tests/rlm_storage_test.rs`.
        // This is the row that the post-disconnect `claim rlm_job` path must
        // be able to flip to `claimed` — the operation that failed with
        // gRPC Internal until the server was restarted (issue
        // agnostic-rlm-rs-ccc3).
        let job = NewRlmJob {
            buffer_id: Some(1),
            project: "proj".to_string(),
            level: 1,
            subject: "src/a.rs".to_string(),
            payload: "{}".to_string(),
            priority: 5,
            quorum_slots: 1,
        };
        let (job_id, _gen) = storage.enqueue_rlm_job(&job).unwrap();
        assert!(job_id > 0, "pending rlm job must be seeded");

        // Stream yields Init + one File then the sender is dropped (simulated
        // mid-index client disconnect during Phase 1/Phase 2).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(4);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk("src/a.rs", "fn main() {}\n")))
            .await
            .unwrap();
        drop(tx);
        let stream = ReceiverStream::new(rx);

        let resp = index_stream_loop(&state, stream, None).await;
        assert!(resp.is_ok(), "handler must return cleanly on disconnect");

        // The claim must SUCCEED (Ok(Some(job))), proving no held
        // connection/transaction from the aborted index leaks into the RLM
        // claim path. An `Err` here reproduces the original bug.
        let claimed = storage.claim_rlm_job("worker", DEFAULT_RLM_LEASE_MS, None);
        assert!(
            claimed.is_ok(),
            "rlm claim must not error after disconnect: {:?}",
            claimed.err()
        );
        assert!(
            claimed.unwrap().is_some(),
            "rlm claim must return the pending job (not None) after disconnect"
        );
    }

    /// Issue `agnostic-rlm-rs-6690`: index embedding must run on a *capped*
    /// rayon pool (built in `AppState`) so a large `arags index` cannot
    /// saturate every core and starve a concurrent `arags search`. This test
    /// drives `index_stream_loop` with a real (but weight-free) lightweight
    /// embedder and a real vector store, asserting the capped pool actually
    /// embeds every chunk with the correct dimensionality.
    #[tokio::test]
    async fn index_embeds_on_capped_pool_with_lightweight_embedder() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();

        let vdir = tempfile::tempdir().unwrap();
        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(
            VectorStore::open_with_dims(vdir.path(), dims)
                .await
                .unwrap(),
        );

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;
        cfg.index_embed_threads = 2;

        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state = AppState::with_embedder(
            storage.clone(),
            cfg,
            embedder,
            Some(vector_store.clone()),
            None,
            None,
            None,
        )
        .unwrap();

        // The capped pool is wired with the configured size.
        assert_eq!(state.index_embed_threads(), 2);

        let files = [
            ("src/a.rs", "fn alpha() {}\nfn beta() {}\n"),
            ("src/b.rs", "fn gamma() {}\n"),
        ];
        let expected: usize = files
            .iter()
            .map(|(p, c)| {
                crate::indexing::index_file_with(
                    Path::new(p),
                    c,
                    tokens_to_lines(state.config.embedder.max_tokens),
                    tokens_to_lines(state.config.embedder.overlap_tokens),
                )
                .len()
            })
            .sum();
        assert!(expected > 0, "fixture must produce chunks");

        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        for (p, c) in &files {
            tx.send(Ok(file_chunk(p, c))).await.unwrap();
        }
        drop(tx);
        let stream = ReceiverStream::new(rx);

        let resp = index_stream_loop(&state, stream, None).await;
        assert!(
            resp.is_ok(),
            "index must succeed with capped pool + lightweight embedder"
        );

        let count = vector_store.count().await;
        assert_eq!(
            count, expected,
            "every chunk must be embedded into the store"
        );
        assert_eq!(
            vector_store.dimensions(),
            dims,
            "stored vectors keep the model dimensionality"
        );
    }

    /// Issue `agnostic-rlm-rs-6690`: while an index embed is in flight on the
    /// *capped* pool, a concurrent query embed on the *global* pool must still
    /// complete promptly (never hang/starve for 90s). This is the structural
    /// guarantee that the bounded pool isolates index work from serving.
    #[tokio::test]
    async fn index_embed_backpressure_keeps_query_serving() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        let vdir = tempfile::tempdir().unwrap();
        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(
            VectorStore::open_with_dims(vdir.path(), dims)
                .await
                .unwrap(),
        );

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;
        cfg.index_embed_threads = 1;

        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state = AppState::with_embedder(
            storage.clone(),
            cfg,
            embedder,
            Some(vector_store),
            None,
            None,
            None,
        )
        .unwrap();

        // Drive a moderately large index on the capped pool in the background.
        let files: Vec<(String, &str)> = (0..40)
            .map(|i| (format!("src/m{i}.rs"), "fn f() {}\n"))
            .collect();
        let bg_state = state.clone();
        let bg = tokio::spawn(async move {
            let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(256);
            tx.send(Ok(init_chunk("proj"))).await.unwrap();
            for (p, c) in &files {
                tx.send(Ok(file_chunk(p, c))).await.unwrap();
            }
            drop(tx);
            let stream = ReceiverStream::new(rx);
            index_stream_loop(&bg_state, stream, None).await
        });

        // Concurrently, hammer the global-pool query embed and assert each
        // finishes well under the old 90s timeout (bounded by a 30s safety net).
        let q_state = state.clone();
        let queries = tokio::spawn(async move {
            for q in 0..50 {
                let st = q_state.clone();
                let r = tokio::time::timeout(Duration::from_secs(30), async move {
                    tokio::task::spawn_blocking(move || st.embedder.embed(&format!("q{q}")))
                        .await
                        .unwrap()
                        .unwrap()
                })
                .await;
                assert!(r.is_ok(), "query embed #{q} must not hang/starve");
            }
        });
        assert!(queries.await.is_ok(), "concurrent query embeds completed");
        assert!(bg.await.unwrap().is_ok(), "background index completed");
    }

    /// Issue `agnostic-rlm-rs-20cd`: a re-index must *replace*, not *append*.
    /// Historically each re-index doubled the chunk/FTS/vector counts
    /// (O(2^n) growth). The deferred Phase-0 delete (e5d0) must keep counts
    /// stable across repeated index streams for the same buffer. This test
    /// feeds DIFFERENT file content per run so it can distinguish runs and
    /// prove the old content is purged.
    #[tokio::test]
    async fn reindex_replaces_chunks_without_duplication() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        seed_project(&storage, "proj", 1);

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;
        let state =
            AppState::with_vector_stores(storage.clone(), cfg, None, None, None, None).unwrap();

        // Run 1: index a single file "a.rs" with a unique marker.
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk("src/a.rs", "fn alpha_marker() {}\n")))
            .await
            .unwrap();
        drop(tx);
        let resp = index_stream_loop(&state, ReceiverStream::new(rx), None).await;
        assert!(resp.is_ok(), "run1 must succeed");

        assert_eq!(
            chunk_count(&storage, 1),
            1,
            "run1 should persist exactly one chunk"
        );
        assert!(
            chunk_text_has(&storage, 1, "alpha_marker"),
            "run1 content must be present in chunk_texts"
        );

        // Run 2: re-index with a DIFFERENT file "b.rs" (replace, not append).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk("src/b.rs", "fn beta_marker() {}\n")))
            .await
            .unwrap();
        drop(tx);
        let resp = index_stream_loop(&state, ReceiverStream::new(rx), None).await;
        assert!(resp.is_ok(), "run2 must succeed");

        assert_eq!(
            chunk_count(&storage, 1),
            1,
            "re-index must replace, not append (no duplication)"
        );
        assert!(
            !chunk_text_has(&storage, 1, "alpha_marker"),
            "run1 content must be gone after re-index"
        );
        assert!(
            chunk_text_has(&storage, 1, "beta_marker"),
            "run2 content must be present"
        );

        // Run 3: re-index again to prove stability (no O(2^n) growth).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk("src/c.rs", "fn gamma_marker() {}\n")))
            .await
            .unwrap();
        drop(tx);
        let resp = index_stream_loop(&state, ReceiverStream::new(rx), None).await;
        assert!(resp.is_ok(), "run3 must succeed");

        assert_eq!(
            chunk_count(&storage, 1),
            1,
            "third re-index must stay stable"
        );
        assert!(
            chunk_text_has(&storage, 1, "gamma_marker"),
            "run3 content must be present"
        );
        assert!(
            !chunk_text_has(&storage, 1, "beta_marker"),
            "run2 content gone after third re-index"
        );
    }

    /// Issue `agnostic-rlm-rs-8dcc`: re-indexing supersedes the previous chunk
    /// version (soft `is_active = 0`) rather than deleting it. The old content
    /// must be retained as history (`is_active = 0`) but MUST NOT surface in
    /// active counts nor in semantic search (its vector is purged at retire).
    #[tokio::test]
    async fn reindex_supersedes_old_chunk_history_retained() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        let vdir = tempfile::tempdir().unwrap();
        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(
            VectorStore::open_with_dims(vdir.path(), dims)
                .await
                .unwrap(),
        );

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;
        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state = AppState::with_embedder(
            storage.clone(),
            cfg,
            embedder,
            Some(vector_store.clone()),
            None,
            None,
            None,
        )
        .unwrap();

        // Run 1: index "src/a.rs" with a unique alpha marker (plus shared text).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk(
            "src/a.rs",
            "fn alpha_marker() {}\nfn keep() {}\n",
        )))
        .await
        .unwrap();
        drop(tx);
        assert!(
            index_stream_loop(&state, ReceiverStream::new(rx), None)
                .await
                .is_ok(),
            "run1 must succeed"
        );

        // Run 2: re-index the SAME file with different content (beta marker).
        let (tx, rx) = mpsc::channel::<Result<IndexChunk, tonic::Status>>(8);
        tx.send(Ok(init_chunk("proj"))).await.unwrap();
        tx.send(Ok(file_chunk(
            "src/a.rs",
            "fn beta_marker() {}\nfn keep() {}\n",
        )))
        .await
        .unwrap();
        drop(tx);
        assert!(
            index_stream_loop(&state, ReceiverStream::new(rx), None)
                .await
                .is_ok(),
            "run2 must succeed"
        );

        // Exactly one ACTIVE chunk (the new version).
        assert_eq!(
            chunk_count(&storage, 1),
            1,
            "exactly one active chunk after supersede"
        );
        // History retained: the superseded run-1 chunk is still present but
        // inactive.
        let retired: i64 = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(
                    c.query_row("SELECT COUNT(*) FROM chunks WHERE is_active = 0", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert!(retired > 0, "superseded chunk history must be retained");

        // The vector store holds only the active chunk: the retired chunk's
        // vector was purged at retire time.
        assert_eq!(
            vector_store.count().await,
            1,
            "only the active vector remains in the store"
        );

        // Fetch the retired chunk's id + text, embed it, and confirm semantic
        // search never returns the retired id (its vector is gone).
        let (retired_id, retired_text) = storage
            .connection()
            .unwrap()
            .execute(|c| {
                let mut stmt = c.prepare(
                    "SELECT c.id, ct.content FROM chunks c \
                     JOIN chunk_texts ct ON ct.chunk_id = c.id WHERE c.is_active = 0 LIMIT 1",
                )?;
                let row =
                    stmt.query_row([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
                Ok(row)
            })
            .unwrap();

        let q_vec = tokio::task::spawn_blocking({
            let st = state.clone();
            let t = retired_text.clone();
            move || st.embedder.embed(&t)
        })
        .await
        .unwrap()
        .unwrap();

        let results = vector_store
            .search_similar(&q_vec, Some(1), 10)
            .await
            .unwrap();
        assert!(
            !results.iter().any(|r| r.chunk_id == retired_id as u64),
            "retired chunk must not surface in semantic search"
        );
    }

    /// Issue `agnostic-rlm-rs-6690` / `agnostic-rlm-rs-5124`: full external
    /// reproduction — run `arags index` of a large repo while `arags search
    /// --tier auto` runs, asserting search latency stays under threshold (no
    /// 90s timeout). Marked `#[ignore]` so CI does not hang on real candle
    /// weights + a large corpus; run manually:
    ///
    /// ```text
    /// ARAGS_INDEX_EMBED_THREADS=2 cargo run --bin arags-server &
    /// arags index <large-repo> &
    /// time arags search --tier auto "<query>"   # must finish < 90s
    /// ```
    #[tokio::test]
    #[ignore = "load"]
    async fn load_regression_index_does_not_starve_search() {
        // Bounded stand-in: a real `arags index` saturates the capped pool via
        // `state.index_embed_pool`; a `search` query embed runs on the global
        // pool. The non-ignored `index_embed_backpressure_keeps_query_serving`
        // test exercises the same isolation with the lightweight embedder.
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        let vdir = tempfile::tempdir().unwrap();
        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(
            VectorStore::open_with_dims(vdir.path(), dims)
                .await
                .unwrap(),
        );
        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;
        cfg.index_embed_threads = num_cpus::get().saturating_sub(2).max(1);
        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state =
            AppState::with_embedder(storage, cfg, embedder, Some(vector_store), None, None, None)
                .unwrap();
        // Smoke: the capped pool is sized to reserve serving cores.
        assert!(state.index_embed_threads() < num_cpus::get() || num_cpus::get() <= 2);
    }
}
