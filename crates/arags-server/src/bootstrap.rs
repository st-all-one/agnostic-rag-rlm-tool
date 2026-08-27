//! Bootstrap rebuild of the four vector spaces from canonical SQLite text
//! (issue `agnostic-rlm-rs-620d`, plan `pl-783b` step 3).
//!
//! On startup the server compares the canonical row count in SQLite (the source
//! of truth) against the vector count currently on disk for each of the four
//! `usearch` spaces (chunks, QA questions, RLM summaries, explorations). When a
//! store is missing or the counts diverge, the space is **rebuilt**: every
//! canonical `(id, text)` pair is re-embedded in bounded batches on the
//! capped index-embed rayon pool and re-inserted, fully replacing the stale
//! index. Spaces whose counts already match are left untouched (best-effort
//! optimization, never a consistency requirement — the periodic flush and the
//! reconcile worker in `36ae` are the other safety nets).
//!
//! DB access is always scoped inside [`crate::store::blocking`]: the canonical
//! inputs are read, the connection is dropped, the (CPU/await) embed runs
//! outside any lock, then vectors are written and the store is persisted.

use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, instrument, warn};

use arags_embedding::embedder::Embedding;
use arags_storage::VectorEntry;

use crate::state::AppState;
use crate::store;

/// Batch size for re-embedding during a bootstrap rebuild. Bounds peak memory
/// of the text + vector buffers so a large divergence cannot OOM the server.
const BOOTSTRAP_BATCH: usize = 512;

/// Per-space bootstrap outcome.
#[derive(Debug, Clone, Default)]
pub struct SpaceReport {
    /// Whether the space was rebuilt (counts diverged or store was missing).
    pub rebuilt: bool,
    /// Canonical SQLite row count for the space.
    pub sqlite_count: u64,
    /// Vector count in the store after bootstrap (post-rebuild or current).
    pub vector_count: u64,
    /// Wall-clock duration for this space in milliseconds.
    pub elapsed_ms: u64,
    /// Whether the store was unavailable so the space could not be rebuilt.
    pub skipped: bool,
}

/// Aggregate bootstrap report.
#[derive(Debug, Clone, Default)]
pub struct BootstrapReport {
    /// Chunk vector space.
    pub chunks: SpaceReport,
    /// QA question vector space.
    pub qa: SpaceReport,
    /// RLM summary vector space.
    pub rlm: SpaceReport,
    /// Exploration-map vector space.
    pub explorations: SpaceReport,
    /// Wall-clock duration of the whole pass in milliseconds.
    pub elapsed_ms: u64,
}

/// Compare the four vector spaces against canonical SQLite and rebuild any that
/// diverge. Best-effort: a per-space failure is logged and recorded in the
/// report, never fatal to server startup. Returns the aggregate report.
///
/// # Errors
///
/// Returns an error only if the bootstrap pass cannot start at all (e.g. the
/// storage handle is unusable). Per-space failures are recorded in
/// [`BootstrapReport`], not returned.
#[instrument(skip_all, fields(phase = "bootstrap_vector_spaces"))]
pub async fn bootstrap_vector_spaces(state: &AppState) -> Result<BootstrapReport> {
    let start = Instant::now();
    let mut report = BootstrapReport::default();

    report.chunks = bootstrap_chunks(state).await;
    report.qa = bootstrap_qa(state).await;
    report.rlm = bootstrap_rlm(state).await;
    report.explorations = bootstrap_explorations(state).await;

    report.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "bootstrap_vector_spaces",
        elapsed_ms = report.elapsed_ms,
        chunks_rebuilt = report.chunks.rebuilt,
        chunks_sqlite = report.chunks.sqlite_count,
        chunks_vector = report.chunks.vector_count,
        qa_rebuilt = report.qa.rebuilt,
        qa_sqlite = report.qa.sqlite_count,
        qa_vector = report.qa.vector_count,
        rlm_rebuilt = report.rlm.rebuilt,
        rlm_sqlite = report.rlm.sqlite_count,
        rlm_vector = report.rlm.vector_count,
        explorations_rebuilt = report.explorations.rebuilt,
        explorations_sqlite = report.explorations.sqlite_count,
        explorations_vector = report.explorations.vector_count,
        "bootstrap_vector_spaces completed"
    );
    Ok(report)
}

/// Rebuild the chunk vector space when its count diverges from SQLite.
async fn bootstrap_chunks(state: &AppState) -> SpaceReport {
    let space = "chunks";
    let start = Instant::now();
    let mut rep = SpaceReport::default();

    let Some(vs) = &state.vector_store else {
        warn!(
            space,
            "vector store unavailable; cannot bootstrap chunk space"
        );
        rep.skipped = true;
        return rep;
    };

    let inputs = match store::blocking({
        let storage = state.storage.clone();
        move || storage.all_chunk_embed_inputs()
    })
    .await
    {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, space, "failed to read canonical chunk inputs");
            rep.skipped = true;
            return rep;
        }
    };

    let sqlite_count = u64::try_from(inputs.len()).unwrap_or(u64::MAX);
    let vector_count = vs.count().await as u64;

    if sqlite_count == vector_count {
        rep.sqlite_count = sqlite_count;
        rep.vector_count = vector_count;
        rep.elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            phase = "bootstrap",
            space,
            status = "in_sync",
            elapsed_ms = rep.elapsed_ms,
            count = sqlite_count,
            "chunk vector space in sync"
        );
        return rep;
    }

    if let Err(e) = vs.clear().await {
        warn!(error = %e, space, "failed to clear chunk vector store");
        rep.skipped = true;
        return rep;
    }

    let mut entries = Vec::with_capacity(inputs.len());
    for batch in inputs.chunks(BOOTSTRAP_BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, _, t)| t.clone()).collect();
        let vectors = match embed_batch(state, &texts).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, space, "chunk bootstrap embed failed; stopping rebuild");
                break;
            }
        };
        for ((chunk_id, buf_id, _), vec) in batch.iter().zip(vectors.iter()) {
            entries.push(VectorEntry {
                chunk_id: u64::try_from(*chunk_id).unwrap_or(u64::MAX),
                buffer_id: u64::try_from(*buf_id).unwrap_or(u64::MAX),
                vector: vec.clone(),
            });
        }
    }

    if let Err(e) = vs.insert_vectors(&entries).await {
        warn!(error = %e, space, "chunk bootstrap insert failed");
    }

    rep.rebuilt = true;
    rep.sqlite_count = sqlite_count;
    rep.vector_count = vs.count().await as u64;
    rep.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "bootstrap",
        space,
        status = "rebuilt",
        elapsed_ms = rep.elapsed_ms,
        sqlite_count,
        vector_count = rep.vector_count,
        "chunk vector space rebuilt from sqlite"
    );
    rep
}

/// Generic rebuild for the `(id, text)` secondary spaces (QA / RLM /
/// explorations). Clears the stale store, re-embeds every canonical pair in
/// bounded batches and re-inserts, then persists.
async fn rebuild_simple_space(
    state: &AppState,
    space: &str,
    inputs: &[(i64, String)],
    clear: impl Fn() -> anyhow::Result<()>,
    insert_one: impl Fn(i64, &Embedding) -> anyhow::Result<()>,
    persist: impl Fn() -> anyhow::Result<()>,
) {
    if let Err(e) = clear() {
        warn!(error = %e, space, "failed to clear vector store; continuing with upsert");
    }
    for batch in inputs.chunks(BOOTSTRAP_BATCH) {
        let texts: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
        let vectors = match embed_batch(state, &texts).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, space, "bootstrap embed failed; stopping rebuild");
                break;
            }
        };
        for ((id, _), vec) in batch.iter().zip(vectors.iter()) {
            if let Err(e) = insert_one(*id, vec) {
                warn!(error = %e, row_id = id, space, "bootstrap vector insert failed");
            }
        }
    }
    if let Err(e) = persist() {
        warn!(error = %e, space, "bootstrap persist failed");
    }
}

/// Rebuild the QA question vector space when its count diverges from SQLite.
async fn bootstrap_qa(state: &AppState) -> SpaceReport {
    let space = "qa";
    let start = Instant::now();
    let mut rep = SpaceReport::default();

    let Some(vs) = &state.question_vector_store else {
        warn!(space, "question vector store unavailable; cannot bootstrap");
        rep.skipped = true;
        return rep;
    };

    let inputs = match store::blocking({
        let storage = state.storage.clone();
        move || storage.all_qa_embed_inputs()
    })
    .await
    {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, space, "failed to read canonical qa inputs");
            rep.skipped = true;
            return rep;
        }
    };

    let sqlite_count = u64::try_from(inputs.len()).unwrap_or(u64::MAX);
    let vector_count = vs.len();

    if sqlite_count == vector_count {
        rep.sqlite_count = sqlite_count;
        rep.vector_count = vector_count;
        rep.elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            phase = "bootstrap",
            space,
            status = "in_sync",
            elapsed_ms = rep.elapsed_ms,
            count = sqlite_count,
            "qa vector space in sync"
        );
        return rep;
    }

    rebuild_simple_space(
        state,
        space,
        &inputs,
        || vs.clear(),
        |id, vec| vs.insert(id as u64, vec),
        || vs.persist(),
    )
    .await;

    rep.rebuilt = true;
    rep.sqlite_count = sqlite_count;
    rep.vector_count = vs.len();
    rep.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "bootstrap",
        space,
        status = "rebuilt",
        elapsed_ms = rep.elapsed_ms,
        sqlite_count,
        vector_count = rep.vector_count,
        "qa vector space rebuilt from sqlite"
    );
    rep
}

/// Rebuild the RLM summary vector space when its count diverges from SQLite.
async fn bootstrap_rlm(state: &AppState) -> SpaceReport {
    let space = "rlm";
    let start = Instant::now();
    let mut rep = SpaceReport::default();

    let Some(vs) = &state.rlm_vector_store else {
        warn!(space, "rlm vector store unavailable; cannot bootstrap");
        rep.skipped = true;
        return rep;
    };

    let inputs = match store::blocking({
        let storage = state.storage.clone();
        move || storage.all_rlm_embed_inputs()
    })
    .await
    {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, space, "failed to read canonical rlm inputs");
            rep.skipped = true;
            return rep;
        }
    };

    let sqlite_count = u64::try_from(inputs.len()).unwrap_or(u64::MAX);
    let vector_count = vs.len();

    if sqlite_count == vector_count {
        rep.sqlite_count = sqlite_count;
        rep.vector_count = vector_count;
        rep.elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            phase = "bootstrap",
            space,
            status = "in_sync",
            elapsed_ms = rep.elapsed_ms,
            count = sqlite_count,
            "rlm vector space in sync"
        );
        return rep;
    }

    rebuild_simple_space(
        state,
        space,
        &inputs,
        || vs.clear(),
        |id, vec| vs.insert(id as u64, vec),
        || vs.persist(),
    )
    .await;

    rep.rebuilt = true;
    rep.sqlite_count = sqlite_count;
    rep.vector_count = vs.len();
    rep.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "bootstrap",
        space,
        status = "rebuilt",
        elapsed_ms = rep.elapsed_ms,
        sqlite_count,
        vector_count = rep.vector_count,
        "rlm vector space rebuilt from sqlite"
    );
    rep
}

/// Rebuild the exploration vector space when its count diverges from SQLite.
async fn bootstrap_explorations(state: &AppState) -> SpaceReport {
    let space = "explorations";
    let start = Instant::now();
    let mut rep = SpaceReport::default();

    let Some(vs) = &state.exploration_vector_store else {
        warn!(
            space,
            "exploration vector store unavailable; cannot bootstrap"
        );
        rep.skipped = true;
        return rep;
    };

    let inputs = match store::blocking({
        let storage = state.storage.clone();
        move || storage.all_exploration_embed_inputs()
    })
    .await
    {
        Ok(i) => i,
        Err(e) => {
            warn!(error = %e, space, "failed to read canonical exploration inputs");
            rep.skipped = true;
            return rep;
        }
    };

    let sqlite_count = u64::try_from(inputs.len()).unwrap_or(u64::MAX);
    let vector_count = vs.len();

    if sqlite_count == vector_count {
        rep.sqlite_count = sqlite_count;
        rep.vector_count = vector_count;
        rep.elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            phase = "bootstrap",
            space,
            status = "in_sync",
            elapsed_ms = rep.elapsed_ms,
            count = sqlite_count,
            "exploration vector space in sync"
        );
        return rep;
    }

    rebuild_simple_space(
        state,
        space,
        &inputs,
        || vs.clear(),
        |id, vec| vs.insert(id as u64, vec),
        || vs.persist(),
    )
    .await;

    rep.rebuilt = true;
    rep.sqlite_count = sqlite_count;
    rep.vector_count = vs.len();
    rep.elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        phase = "bootstrap",
        space,
        status = "rebuilt",
        elapsed_ms = rep.elapsed_ms,
        sqlite_count,
        vector_count = rep.vector_count,
        "exploration vector space rebuilt from sqlite"
    );
    rep
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
            active.fetch_add(1, Ordering::SeqCst);
            let r = embedder.embed_batch(&refs);
            active.fetch_sub(1, Ordering::SeqCst);
            r
        })
    })
    .await
    .context("embedding task panicked")?
    .map_err(|e| anyhow::anyhow!("embedding failed: {e}"))
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
    use arags_embedding::embedder::lightweight::LightweightEmbedder;

    use crate::config::ServerConfig;
    use crate::state::AppState;
    use arags_storage::ExplorationVectorStore;
    use arags_storage::QuestionVectorStore;
    use arags_storage::RlmVectorStore;
    use arags_storage::Storage;
    use arags_storage::VectorStore;

    async fn fixture_with_all_stores() -> (
        tempfile::TempDir,
        Storage,
        AppState,
        Arc<VectorStore>,
        Arc<QuestionVectorStore>,
        Arc<RlmVectorStore>,
        Arc<ExplorationVectorStore>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).unwrap();

        let dims = arags_embedding::embedder::minilm::HIDDEN_SIZE;
        let chunk_vs = Arc::new(VectorStore::open_with_dims(dir.path(), dims).await.unwrap());
        let qv = Arc::new(QuestionVectorStore::open(dir.path(), dims).unwrap());
        let rv = Arc::new(RlmVectorStore::open(dir.path(), dims).unwrap());
        let ev = Arc::new(ExplorationVectorStore::open(dir.path(), dims).unwrap());

        let embedder: Arc<dyn Embedder + Send + Sync> = Arc::new(LightweightEmbedder::new(dims));
        let state = AppState::with_embedder(
            storage.clone(),
            ServerConfig::default(),
            embedder,
            Some(chunk_vs.clone()),
            Some(qv.clone()),
            Some(rv.clone()),
            Some(ev.clone()),
        )
        .unwrap();
        (dir, storage, state, chunk_vs, qv, rv, ev)
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
                     VALUES (?1, ?2, 'f.rs', 0, 1, 1, 1, ?3, 'active')",
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
    async fn bootstrap_rebuilds_divergent_chunk_space_from_sqlite() {
        let (_dir, storage, state, chunk_vs, _qv, _rv, _ev) = fixture_with_all_stores().await;
        seed_buffer(&storage, 1, "proj");
        seed_chunk(&storage, 10, 1, "fn alpha() {}");
        seed_chunk(&storage, 11, 1, "fn beta() {}");

        // Divergence: SQLite has 2 canonical chunks but the store is empty.
        assert_eq!(chunk_vs.count().await, 0);

        let report = crate::bootstrap::bootstrap_vector_spaces(&state)
            .await
            .unwrap();

        assert!(report.chunks.rebuilt);
        assert_eq!(report.chunks.sqlite_count, 2);
        assert_eq!(report.chunks.vector_count, 2);
        assert_eq!(chunk_vs.count().await, 2);

        // A previously-missing vector is now searchable.
        let q = state.embedder.embed("fn alpha() {}").unwrap();
        let results = chunk_vs.search_similar(&q, Some(1), 5).await.unwrap();
        assert!(
            results.iter().any(|r| r.chunk_id == 10),
            "rebuilt chunk 10 must be searchable"
        );
    }

    #[tokio::test]
    async fn bootstrap_skips_in_sync_space() {
        let (_dir, storage, state, chunk_vs, _qv, _rv, _ev) = fixture_with_all_stores().await;
        seed_buffer(&storage, 1, "proj");
        seed_chunk(&storage, 10, 1, "fn alpha() {}");
        seed_chunk(&storage, 11, 1, "fn beta() {}");

        // Pre-populate the store so counts match (in sync).
        let inputs = storage.all_chunk_embed_inputs().unwrap();
        let texts: Vec<String> = inputs.iter().map(|(_, _, t)| t.clone()).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let vecs = state.embedder.embed_batch(&refs).unwrap();
        let entries: Vec<_> = inputs
            .iter()
            .zip(vecs.iter())
            .map(|((cid, bid, _), v)| arags_storage::VectorEntry {
                chunk_id: u64::try_from(*cid).unwrap(),
                buffer_id: u64::try_from(*bid).unwrap(),
                vector: v.clone(),
            })
            .collect();
        chunk_vs.insert_vectors(&entries).await.unwrap();
        assert_eq!(chunk_vs.count().await, 2);

        let report = crate::bootstrap::bootstrap_vector_spaces(&state)
            .await
            .unwrap();

        assert!(!report.chunks.rebuilt, "in-sync space must not be rebuilt");
        assert_eq!(report.chunks.sqlite_count, 2);
        assert_eq!(report.chunks.vector_count, 2);
        assert_eq!(chunk_vs.count().await, 2);
    }

    #[tokio::test]
    async fn bootstrap_rebuilds_all_four_spaces_when_empty() {
        let (_dir, storage, state, chunk_vs, qv, rv, ev) = fixture_with_all_stores().await;
        seed_buffer(&storage, 1, "proj");

        storage
            .connection()
            .unwrap()
            .execute(|c| {
                c.execute(
                    "INSERT INTO chunks \
                     (id, buffer_id, file_path, offset_start, offset_end, line_start, line_end, hash, status) \
                     VALUES (10, 1, 'f.rs', 0, 1, 1, 1, X'61', 'active')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO chunk_texts (chunk_id, content) VALUES (10, 'fn alpha() {}')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO qa_cache \
                     (id, cache_id, buffer_id, project, question_text, question_hash, answer_text, created_at, last_accessed_at, vector_status) \
                     VALUES (20, 'c1', 1, 'proj', 'how does x work', 'h1', 'a', 0, 0, 'indexed')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO rlm_nodes \
                     (id, node_id, buffer_id, project, level, subject, summary_text, created_at, updated_at, last_accessed_at, vector_status) \
                     VALUES (30, 'n1', 1, 'proj', 1, 'f.rs', 'summary', 0, 0, 0, 'indexed')",
                    [],
                )?;
                c.execute(
                    "INSERT INTO explorations \
                     (id, exploration_id, project, buffer_id, goal, body, summary, created_by, created_at, updated_at, last_accessed_at, vector_status) \
                     VALUES (40, 'e1', 'proj', 1, 'goal', X'00', 'summary', 'u', 0, 0, 0, 'indexed')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        // All four stores start empty.
        assert_eq!(chunk_vs.count().await, 0);
        assert_eq!(qv.len(), 0);
        assert_eq!(rv.len(), 0);
        assert_eq!(ev.len(), 0);

        let report = crate::bootstrap::bootstrap_vector_spaces(&state)
            .await
            .unwrap();

        assert!(report.chunks.rebuilt);
        assert!(report.qa.rebuilt);
        assert!(report.rlm.rebuilt);
        assert!(report.explorations.rebuilt);

        assert_eq!(report.chunks.sqlite_count, 1);
        assert_eq!(report.chunks.vector_count, 1);
        assert_eq!(report.qa.sqlite_count, 1);
        assert_eq!(report.qa.vector_count, 1);
        assert_eq!(report.rlm.sqlite_count, 1);
        assert_eq!(report.rlm.vector_count, 1);
        assert_eq!(report.explorations.sqlite_count, 1);
        assert_eq!(report.explorations.vector_count, 1);
    }
}
