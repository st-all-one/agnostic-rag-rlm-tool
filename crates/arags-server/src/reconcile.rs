//! Reconcile worker: re-derive missing usearch vectors from canonical SQLite
//! text (issue `agnostic-rlm-rs-36ae`, plan `pl-783b` step 2).
//!
//! When an embedding or vector insert fails during normal indexing/store, the
//! row is recorded with a `pending_vector` status (issue `agnostic-rlm-rs-50ed`)
//! so the data plane stays consistent: SQLite remains the source of truth and
//! the semantic spaces eventually catch up. This module walks the four vector
//! spaces (chunks, QA questions, RLM summaries, explorations), re-embeds the
//! canonical text for every pending row and upserts the vector into the
//! matching `usearch` space, then clears the marker. Gap metrics (pending
//! before, processed, still failing) are reported and logged.
//!
//! DB access is always scoped inside [`crate::store::blocking`]: pending ids are
//! read, the connection is dropped, the (CPU/await) embed runs outside any lock,
//! then vectors are written and markers cleared inside a fresh blocking scope.
//! Embedding runs on the capped index-embed rayon pool (issue
//! `agnostic-rlm-rs-6690`) so reconcile never saturates the global pool.

use std::sync::Arc;
use std::time::Instant;
use tracing::{info, instrument, warn};

use anyhow::{Context, Result};

use arags_embedding::embedder::Embedding;
use arags_storage::ExplorationVectorStore;
use arags_storage::QuestionVectorStore;
use arags_storage::RlmVectorStore;
use arags_storage::VectorEntry;
use arags_storage::VectorStore;

use crate::state::AppState;
use crate::store;

/// Maximum number of pending rows re-derived per space per maintenance tick
/// page. Bounds peak memory of the text+vector buffers so a burst of failures
/// cannot OOM the server; the next tick continues where this one stopped.
const RECONCILE_BATCH: usize = 512;

/// Per-space reconcile counters (gap metrics).
#[derive(Debug, Clone, Default)]
pub struct SpaceReconcile {
    /// Pending rows observed before this pass.
    pub pending_before: u64,
    /// Rows successfully re-embedded and re-inserted.
    pub processed: u64,
    /// Rows still failing (no canonical text, embed failure, or insert failure).
    pub remaining: u64,
}

impl SpaceReconcile {
    fn merge(&mut self, other: &SpaceReconcile) {
        self.pending_before += other.pending_before;
        self.processed += other.processed;
        self.remaining += other.remaining;
    }
}

/// Aggregate reconcile report (mirrors [`crate::maintenance::MaintenanceReport`]
/// style for consistency with the maintenance ticker).
#[derive(Debug, Clone, Default)]
pub struct ReconcileReport {
    /// Chunk vector space.
    pub chunks: SpaceReconcile,
    /// QA question vector space.
    pub qa: SpaceReconcile,
    /// RLM summary vector space.
    pub rlm: SpaceReconcile,
    /// Exploration-map vector space.
    pub explorations: SpaceReconcile,
    /// Wall-clock duration of the whole pass in milliseconds.
    pub elapsed_ms: u64,
}

/// Re-derive every `pending_vector` row across all four vector spaces, for every
/// project/buffer. Reads the pending ids per space, re-embeds the canonical text
/// outside the SQLite lock, upserts into the matching `usearch` space and clears
/// the marker. Returns gap metrics for observability.
///
/// # Errors
///
/// Returns an error only if a fatal, non-recoverable failure prevents the pass
/// from starting (e.g. the project list cannot be read). Per-row failures are
/// recorded in [`ReconcileReport::remaining`] and never abort the pass.
#[instrument(skip_all, fields(phase = "reconcile_pending_vectors"))]
pub async fn reconcile_pending_vectors(state: &AppState) -> Result<ReconcileReport> {
    let start = Instant::now();
    let mut report = ReconcileReport::default();

    let buffers = store::blocking({
        let storage = state.storage.clone();
        move || store::list_projects(&storage)
    })
    .await
    .context("failed to list projects for reconcile")?;

    for buf in &buffers {
        let buffer_id = buf.id;
        let project = buf.name.clone();

        if let Some(vs) = &state.vector_store {
            report
                .chunks
                .merge(&reconcile_chunks(state, vs, buffer_id).await);
        }
        if let Some(vs) = &state.rlm_vector_store {
            report.rlm.merge(&reconcile_rlm(state, vs, buffer_id).await);
        }
        if let Some(vs) = &state.exploration_vector_store {
            report
                .explorations
                .merge(&reconcile_explorations(state, vs, buffer_id).await);
        }
        if let Some(vs) = &state.question_vector_store {
            report.qa.merge(&reconcile_qa(state, vs, &project).await);
        }
    }

    report.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "reconcile_pending_vectors",
        elapsed_ms = report.elapsed_ms,
        chunks_pending = report.chunks.pending_before,
        chunks_processed = report.chunks.processed,
        chunks_remaining = report.chunks.remaining,
        qa_pending = report.qa.pending_before,
        qa_processed = report.qa.processed,
        qa_remaining = report.qa.remaining,
        rlm_pending = report.rlm.pending_before,
        rlm_processed = report.rlm.processed,
        rlm_remaining = report.rlm.remaining,
        explorations_pending = report.explorations.pending_before,
        explorations_processed = report.explorations.processed,
        explorations_remaining = report.explorations.remaining,
        "reconcile_pending_vectors completed"
    );
    Ok(report)
}

/// Re-derive pending chunk vectors for one buffer.
async fn reconcile_chunks(
    state: &AppState,
    vs: &Arc<VectorStore>,
    buffer_id: i64,
) -> SpaceReconcile {
    let mut acc = SpaceReconcile::default();
    let pending = match store::blocking({
        let storage = state.storage.clone();
        move || storage.chunks_pending_vector(buffer_id)
    })
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, buffer_id, "failed to read pending chunks");
            return acc;
        }
    };

    acc.pending_before = u64::try_from(pending.len()).unwrap_or(u64::MAX);
    for batch in pending.chunks(RECONCILE_BATCH) {
        let ids: Vec<i64> = batch.to_vec();
        let inputs = match store::blocking({
            let storage = state.storage.clone();
            let ids = ids.clone();
            move || storage.get_chunk_embed_inputs(&ids)
        })
        .await
        {
            Ok(i) => i,
            Err(e) => {
                warn!(error = %e, buffer_id, "failed to read chunk embed inputs");
                continue;
            }
        };

        let (cleared, failed) = embed_and_insert_chunks(state, vs, buffer_id, &inputs).await;
        let got: std::collections::HashSet<i64> = inputs.iter().map(|(id, _, _)| *id).collect();
        let missing: Vec<i64> = ids.iter().copied().filter(|id| !got.contains(id)).collect();

        if !cleared.is_empty() {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let cleared = cleared.clone();
                move || storage.clear_chunks_pending_vector(buffer_id, &cleared)
            })
            .await
            {
                warn!(error = %e, buffer_id, "failed to clear chunks pending_vector");
            }
        }

        acc.processed += u64::try_from(cleared.len()).unwrap_or(0);
        acc.remaining += u64::try_from(failed.len() + missing.len()).unwrap_or(0);
    }
    acc
}

/// Re-derive pending QA question vectors for one project.
async fn reconcile_qa(
    state: &AppState,
    vs: &Arc<QuestionVectorStore>,
    project: &str,
) -> SpaceReconcile {
    let mut acc = SpaceReconcile::default();
    let pending = match store::blocking({
        let storage = state.storage.clone();
        let project = project.to_string();
        move || storage.qa_cache_pending_vector(&project)
    })
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, project, "failed to read pending qa_cache");
            return acc;
        }
    };

    acc.pending_before = u64::try_from(pending.len()).unwrap_or(u64::MAX);
    for batch in pending.chunks(RECONCILE_BATCH) {
        let ids: Vec<i64> = batch.to_vec();
        let inputs = match store::blocking({
            let storage = state.storage.clone();
            let ids = ids.clone();
            move || storage.get_qa_embed_inputs(&ids)
        })
        .await
        {
            Ok(i) => i,
            Err(e) => {
                warn!(error = %e, project, "failed to read qa embed inputs");
                continue;
            }
        };

        let (cleared, failed) =
            embed_and_insert(state, &inputs, |id, vec| vs.insert(id as u64, vec)).await;
        let got: std::collections::HashSet<i64> = inputs.iter().map(|(id, _)| *id).collect();
        let missing: Vec<i64> = ids.iter().copied().filter(|id| !got.contains(id)).collect();

        if !cleared.is_empty() {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let cleared = cleared.clone();
                move || storage.clear_qa_cache_pending_vector(&cleared)
            })
            .await
            {
                warn!(error = %e, project, "failed to clear qa_cache pending_vector");
            }
        }

        acc.processed += u64::try_from(cleared.len()).unwrap_or(0);
        acc.remaining += u64::try_from(failed.len() + missing.len()).unwrap_or(0);
    }
    acc
}

/// Re-derive pending RLM summary vectors for one buffer.
async fn reconcile_rlm(
    state: &AppState,
    vs: &Arc<RlmVectorStore>,
    buffer_id: i64,
) -> SpaceReconcile {
    let mut acc = SpaceReconcile::default();
    let pending = match store::blocking({
        let storage = state.storage.clone();
        move || storage.rlm_nodes_pending_vector(buffer_id)
    })
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, buffer_id, "failed to read pending rlm nodes");
            return acc;
        }
    };

    acc.pending_before = u64::try_from(pending.len()).unwrap_or(u64::MAX);
    for batch in pending.chunks(RECONCILE_BATCH) {
        let ids: Vec<i64> = batch.to_vec();
        let inputs = match store::blocking({
            let storage = state.storage.clone();
            let ids = ids.clone();
            move || storage.get_rlm_embed_inputs(&ids)
        })
        .await
        {
            Ok(i) => i,
            Err(e) => {
                warn!(error = %e, buffer_id, "failed to read rlm embed inputs");
                continue;
            }
        };

        let (cleared, failed) =
            embed_and_insert(state, &inputs, |id, vec| vs.insert(id as u64, vec)).await;
        let got: std::collections::HashSet<i64> = inputs.iter().map(|(id, _)| *id).collect();
        let missing: Vec<i64> = ids.iter().copied().filter(|id| !got.contains(id)).collect();

        if !cleared.is_empty() {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let cleared = cleared.clone();
                move || storage.clear_rlm_nodes_pending_vector(buffer_id, &cleared)
            })
            .await
            {
                warn!(error = %e, buffer_id, "failed to clear rlm pending_vector");
            }
        }

        acc.processed += u64::try_from(cleared.len()).unwrap_or(0);
        acc.remaining += u64::try_from(failed.len() + missing.len()).unwrap_or(0);
    }
    acc
}

/// Re-derive pending exploration vectors for one buffer.
async fn reconcile_explorations(
    state: &AppState,
    vs: &Arc<ExplorationVectorStore>,
    buffer_id: i64,
) -> SpaceReconcile {
    let mut acc = SpaceReconcile::default();
    let pending = match store::blocking({
        let storage = state.storage.clone();
        move || storage.explorations_pending_vector(buffer_id)
    })
    .await
    {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, buffer_id, "failed to read pending explorations");
            return acc;
        }
    };

    acc.pending_before = u64::try_from(pending.len()).unwrap_or(u64::MAX);
    for batch in pending.chunks(RECONCILE_BATCH) {
        let ids: Vec<i64> = batch.to_vec();
        let inputs = match store::blocking({
            let storage = state.storage.clone();
            let ids = ids.clone();
            move || storage.get_exploration_embed_inputs(&ids)
        })
        .await
        {
            Ok(i) => i,
            Err(e) => {
                warn!(error = %e, buffer_id, "failed to read exploration embed inputs");
                continue;
            }
        };

        let (cleared, failed) =
            embed_and_insert(state, &inputs, |id, vec| vs.insert(id as u64, vec)).await;
        let got: std::collections::HashSet<i64> = inputs.iter().map(|(id, _)| *id).collect();
        let missing: Vec<i64> = ids.iter().copied().filter(|id| !got.contains(id)).collect();

        if !cleared.is_empty() {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let cleared = cleared.clone();
                move || storage.clear_explorations_pending_vector(buffer_id, &cleared)
            })
            .await
            {
                warn!(error = %e, buffer_id, "failed to clear explorations pending_vector");
            }
        }

        acc.processed += u64::try_from(cleared.len()).unwrap_or(0);
        acc.remaining += u64::try_from(failed.len() + missing.len()).unwrap_or(0);
    }
    acc
}

/// Revert expired QA re-digest leases back to `pending` so the next maintenance
/// cycle re-offers the work to volunteers (issue `agnostic-rlm-rs-d172`). The
/// server is LLM-free: it only owns the queue, not the digest.
///
/// # Errors
///
/// Returns an error if the reclaim pass cannot start; per-row failures are
/// reported in [`PendingQaCounts::expired`] and never abort the pass.
#[instrument(skip_all, fields(phase = "reclaim_expired_pending_qa"))]
pub async fn reclaim_expired_pending_qa(
    state: &AppState,
) -> Result<arags_storage::PendingQaCounts> {
    let start = Instant::now();
    let now = chrono::Utc::now().timestamp();
    let counts = store::blocking({
        let storage = state.storage.clone();
        move || storage.revert_expired_pending_qa(now)
    })
    .await
    .context("failed to reclaim expired pending qa")?;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "reclaim_expired_pending_qa",
        elapsed_ms,
        pending = counts.pending,
        leased = counts.leased,
        completed = counts.completed,
        expired = counts.expired,
        "reclaim_expired_pending_qa completed"
    );
    Ok(counts)
}

/// Embed a batch of canonical texts on the capped index-embed rayon pool.
///
/// # Errors
///
/// Returns an error if the embedding task panics or the embedder fails.
async fn embed_batch(state: &AppState, texts: &[String]) -> Result<Vec<Embedding>> {
    let embedder = state.embedder.clone();
    let pool = state.index_embed_pool.clone();
    let active = state.active_index_embeds.clone();
    let owned: Vec<String> = texts.to_vec();
    tokio::task::spawn_blocking(move || {
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        pool.install(|| {
            active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let r = embedder.embed_batch(&refs);
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            r
        })
    })
    .await
    .context("embedding task panicked")?
    .map_err(|e| anyhow::anyhow!("embedding failed: {e}"))
}

/// Embed chunk inputs and re-insert them into the chunk vector space. Returns
/// `(cleared, failed)` id lists. Chunk insert needs the buffer id, so it is
/// specialized rather than sharing [`embed_and_insert`].
async fn embed_and_insert_chunks(
    state: &AppState,
    vs: &Arc<VectorStore>,
    buffer_id: i64,
    inputs: &[(i64, i64, String)],
) -> (Vec<i64>, Vec<i64>) {
    if inputs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let texts: Vec<String> = inputs.iter().map(|(_, _, t)| t.clone()).collect();
    let vectors = match embed_batch(state, &texts).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, buffer_id, "reconcile chunk embed batch failed; leaving pending");
            return (Vec::new(), inputs.iter().map(|(id, _, _)| *id).collect());
        }
    };
    let mut cleared = Vec::new();
    let mut failed = Vec::new();
    for ((chunk_id, buf_id, _), vec) in inputs.iter().zip(vectors.iter()) {
        let entry = VectorEntry {
            chunk_id: u64::try_from(*chunk_id).unwrap_or(u64::MAX),
            buffer_id: u64::try_from(*buf_id).unwrap_or(u64::MAX),
            vector: vec.clone(),
        };
        if let Err(e) = vs.insert_vectors(&[entry]).await {
            warn!(error = %e, chunk_id, "reconcile chunk vector insert failed");
            failed.push(*chunk_id);
        } else {
            cleared.push(*chunk_id);
        }
    }
    (cleared, failed)
}

/// Embed `(id, text)` inputs and re-insert each into a sync `usearch` space via
/// `insert_one`. Returns `(cleared, failed)` id lists; on a full embed-batch
/// failure every input is returned as failed (left pending).
async fn embed_and_insert<F>(
    state: &AppState,
    inputs: &[(i64, String)],
    insert_one: F,
) -> (Vec<i64>, Vec<i64>)
where
    F: Fn(i64, &Embedding) -> Result<()>,
{
    if inputs.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let texts: Vec<String> = inputs.iter().map(|(_, t)| t.clone()).collect();
    let vectors = match embed_batch(state, &texts).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "reconcile embed batch failed; leaving rows pending");
            return (Vec::new(), inputs.iter().map(|(id, _)| *id).collect());
        }
    };

    let mut cleared = Vec::new();
    let mut failed = Vec::new();
    for ((id, _), vec) in inputs.iter().zip(vectors.iter()) {
        if let Err(e) = insert_one(*id, vec) {
            warn!(error = %e, row_id = id, "reconcile vector insert failed");
            failed.push(*id);
        } else {
            cleared.push(*id);
        }
    }
    (cleared, failed)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::cast_possible_truncation
    )]

    use std::sync::Arc;

    use arags_embedding::embedder::Embedder;
    use arags_embedding::embedder::Embedding;
    use arags_embedding::embedder::lightweight::LightweightEmbedder;

    use crate::config::ServerConfig;
    use crate::state::AppState;
    use arags_storage::Storage;
    use arags_storage::VectorStore;

    /// Build a state with a real (weight-free) embedder and a real chunk vector
    /// store, but no QA/RLM/exploration stores (those are exercised by the gap
    /// metric test with a lightweight embedder + in-memory stores).
    async fn fixture() -> (tempfile::TempDir, Storage, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();

        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(VectorStore::open_with_dims(dir.path(), dims).await.unwrap());

        let mut cfg = ServerConfig::default();
        cfg.exploration.enabled = false;
        cfg.rlm.enabled = false;

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
        (dir, storage, state)
    }

    fn seed_buffer(storage: &Storage, id: i64, name: &str) {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO buffers (id, name, path) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![id, name, "/tmp/p"],
                )?;
                Ok(())
            })
            .unwrap();
    }

    fn seed_chunk(storage: &Storage, chunk_id: i64, buffer_id: i64, content: &str) {
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO chunks \
                     (id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, status) \
                     VALUES (?1, ?2, 'f.rs', 0, 1, 1, 1, ?3, 'pending_vector')",
                    rusqlite::params![chunk_id, buffer_id, content.as_bytes()],
                )?;
                c.execute(
                    "INSERT INTO chunk_texts (chunk_id, content) VALUES (?1, ?2)",
                    rusqlite::params![chunk_id, content],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn reconcile_clears_pending_vector_and_inserts_chunk_vector() {
        let (_dir, storage, state) = fixture().await;
        seed_buffer(&storage, 1, "proj");
        seed_chunk(&storage, 10, 1, "fn alpha() {}");
        seed_chunk(&storage, 11, 1, "fn beta() {}");

        // Both chunks are pending before reconcile.
        assert_eq!(storage.chunks_pending_vector(1).unwrap().len(), 2);

        let report = crate::reconcile::reconcile_pending_vectors(&state)
            .await
            .unwrap();

        // Status cleared and vectors present + searchable.
        assert_eq!(storage.chunks_pending_vector(1).unwrap().len(), 0);
        let status: String = storage
            .connection()
            .unwrap()
            .execute(|c| {
                Ok(c.query_row("SELECT status FROM chunks WHERE id = 10", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(status, "active");

        let vs = state.vector_store.as_ref().unwrap();
        assert_eq!(
            vs.count().await,
            2,
            "both re-embedded chunk vectors must be present"
        );

        assert_eq!(report.chunks.pending_before, 2);
        assert_eq!(report.chunks.processed, 2);
        assert_eq!(report.chunks.remaining, 0);
    }

    #[tokio::test]
    async fn reconcile_handles_all_four_spaces_gap_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        seed_buffer(&storage, 1, "proj");

        // Seed pending rows in every space (canonical text present).
        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO chunks \
                     (id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, status) \
                     VALUES (10, 1, 'f.rs', 0, 1, 1, 1, X'61', 'pending_vector')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO chunk_texts (chunk_id, content) VALUES (10, 'fn alpha() {}')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO qa_cache \
                     (id, cache_id, buffer_id, project, question_text, question_hash, answer_text, created_at, last_accessed_at, vector_status) \
                     VALUES (20, 'c1', 1, 'proj', 'how does x work', 'h1', 'a', 0, 0, 'pending_vector')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO rlm_nodes \
                     (id, node_id, buffer_id, project, level, subject, summary_text, created_at, updated_at, last_accessed_at, vector_status) \
                     VALUES (30, 'n1', 1, 'proj', 1, 'f.rs', 'summary', 0, 0, 0, 'pending_vector')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO explorations \
                     (id, exploration_id, project, buffer_id, goal, body, summary, created_by, created_at, updated_at, last_accessed_at, vector_status) \
                     VALUES (40, 'e1', 'proj', 1, 'goal', X'00', 'summary', 'u', 0, 0, 0, 'pending_vector')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let chunk_vs = Arc::new(VectorStore::open_with_dims(dir.path(), dims).await.unwrap());
        let qv = Arc::new(arags_storage::QuestionVectorStore::open(dir.path(), dims).unwrap());
        let rv = Arc::new(arags_storage::RlmVectorStore::open(dir.path(), dims).unwrap());
        let ev = Arc::new(arags_storage::ExplorationVectorStore::open(dir.path(), dims).unwrap());

        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state = AppState::with_embedder(
            storage.clone(),
            ServerConfig::default(),
            embedder,
            Some(chunk_vs),
            Some(qv),
            Some(rv),
            Some(ev),
        )
        .unwrap();

        let report = crate::reconcile::reconcile_pending_vectors(&state)
            .await
            .unwrap();

        assert_eq!(report.chunks.pending_before, 1);
        assert_eq!(report.chunks.processed, 1);
        assert_eq!(report.qa.pending_before, 1);
        assert_eq!(report.qa.processed, 1);
        assert_eq!(report.rlm.pending_before, 1);
        assert_eq!(report.rlm.processed, 1);
        assert_eq!(report.explorations.pending_before, 1);
        assert_eq!(report.explorations.processed, 1);

        // All markers cleared.
        assert_eq!(storage.chunks_pending_vector(1).unwrap().len(), 0);
        assert_eq!(storage.qa_cache_pending_vector("proj").unwrap().len(), 0);
        assert_eq!(storage.rlm_nodes_pending_vector(1).unwrap().len(), 0);
        assert_eq!(storage.explorations_pending_vector(1).unwrap().len(), 0);
    }

    /// Mock embedder that always fails, used to verify the reconcile worker
    /// re-marks (leaves) rows pending on embed failure instead of clearing them.
    struct FailingEmbedder {
        dims: usize,
    }

    impl Embedder for FailingEmbedder {
        fn embed(&self, _text: &str) -> arags_embedding::embedder::EmbeddingResult<Embedding> {
            Err(arags_embedding::embedder::EmbeddingError::Candle(
                "simulated embed failure".into(),
            ))
        }

        fn embed_batch(
            &self,
            _texts: &[&str],
        ) -> arags_embedding::embedder::EmbeddingResult<Vec<Embedding>> {
            Err(arags_embedding::embedder::EmbeddingError::Candle(
                "simulated embed batch failure".into(),
            ))
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        fn name(&self) -> &'static str {
            "failing"
        }
    }

    #[tokio::test]
    async fn reconcile_remarks_pending_on_embed_failure() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();
        seed_buffer(&storage, 1, "proj");
        seed_chunk(&storage, 10, 1, "fn alpha() {}");
        seed_chunk(&storage, 11, 1, "fn beta() {}");

        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let vector_store = Arc::new(VectorStore::open_with_dims(dir.path(), dims).await.unwrap());

        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(FailingEmbedder { dims });
        let state = AppState::with_embedder(
            storage.clone(),
            ServerConfig::default(),
            embedder,
            Some(vector_store),
            None,
            None,
            None,
        )
        .unwrap();

        let report = crate::reconcile::reconcile_pending_vectors(&state)
            .await
            .unwrap();

        // Embed failed for every row, so nothing was cleared and all remain
        // pending (re-marked / untouched).
        assert_eq!(storage.chunks_pending_vector(1).unwrap().len(), 2);
        assert_eq!(report.chunks.pending_before, 2);
        assert_eq!(report.chunks.processed, 0);
        assert_eq!(report.chunks.remaining, 2);
    }
}
