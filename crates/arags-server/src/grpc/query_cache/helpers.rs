//! Shared helpers for the query-answer cache RPCs: embedding, threshold
//! resolution, buffer resolution, and provenance verification.

use arags_core::qa_cache::QaThresholds;
use arags_storage::qa_cache as qa_store;
use tracing::{debug, info, warn};

use crate::grpc::search::{buffer_id_for, hybrid_search};
use crate::grpc::util::sanitize_fts;
use crate::state::AppState;
use crate::store;

use arags_proto::proto::SearchResult;

/// Task prefix applied to question embeddings (separate vector space B).
const QUESTION_PREFIX: &str = "search_query: ";

/// Candidate pool fetched from the question-vector store for the near-hit
/// similarity check.
pub(crate) const NEAR_HIT_CANDIDATES: usize = 10;

/// Embed a question in the dedicated question space (blocking).
///
/// Runs on the **global** rayon pool (not the capped index pool), so a
/// concurrent `arags index` cannot starve QA-cache lookups (issue
/// `agnostic-rag-rlm-tool-6690`).
pub(crate) async fn embed_query(state: &AppState, question: &str) -> Option<Vec<f32>> {
    if state.index_embed_in_flight() > 0 {
        debug!(
            active_index_embeds = state.index_embed_in_flight(),
            "qa query embed contends with active index embed; served on global pool"
        );
    }
    let embedder = state.embedder.clone();
    let text = format!("{QUESTION_PREFIX}{question}");
    tokio::task::spawn_blocking(move || embedder.embed(&text))
        .await
        .ok()
        .and_then(Result::ok)
}

/// Build the adaptive thresholds from server config.
pub(crate) fn thresholds(state: &AppState) -> QaThresholds {
    let c = &state.qa_config;
    QaThresholds {
        novel_k: c.novel_k,
        provenance_k: c.provenance_k,
        sim_high: c.sim_high,
        sim_floor: c.sim_floor,
        tier_steps: c.tier_steps.clone(),
        jaccard_min: c.jaccard_min,
    }
}

/// Resolve a project to its buffer id (explicit or by name).
pub(crate) async fn resolve_buffer(state: &AppState, project: &str, explicit: i64) -> Option<i64> {
    if explicit > 0 {
        return Some(explicit);
    }
    buffer_id_for(state, project).await.ok().flatten()
}

/// Verify that the cached answer's provenance chunks still carry the same
/// content hashes (trust pipeline borrowed from explorations, plan 023).
///
/// Entries without provenance (`source_chunk_ids` empty) are unverifiable and
/// pass through. On drift the entry is marked stale so later queries miss fast.
pub(crate) async fn provenance_intact(state: &AppState, row: &qa_store::QaCacheRow) -> bool {
    if row.source_chunk_ids.is_empty() || row.source_hashes.is_empty() {
        return true;
    }
    let pairs: Vec<(i64, String)> = row
        .source_chunk_ids
        .iter()
        .zip(row.source_hashes.iter())
        .filter_map(|(id, hash)| id.parse::<i64>().ok().map(|i| (i, hash.clone())))
        .collect();
    if pairs.is_empty() {
        return true;
    }
    let storage = state.storage.clone();
    match store::blocking(move || storage.chunk_hashes_match(&pairs)).await {
        Ok(true) => true,
        Ok(false) => {
            let id = row.id;
            let stale_storage = state.storage.clone();
            let _ = store::blocking(move || {
                stale_storage.mark_qa_stale(id, "system", "provenance drift")
            })
            .await;
            info!(cache_id = %row.cache_id, "qa provenance drifted; entry marked stale");
            false
        }
        Err(e) => {
            // Fail open: verification trouble must not break serving.
            warn!(error = %e, "qa provenance check failed; serving anyway");
            true
        }
    }
}

/// Fetch provenance chunks for a cached answer (top `k` by stored order). When
/// `as_of_epoch > 0` (plan 021) each chunk is time-traveled to the revision
/// active at that epoch.
pub(crate) async fn provenance_chunks(
    state: &AppState,
    ids: &[String],
    k: usize,
    as_of_epoch: i64,
) -> Vec<SearchResult> {
    let ids: Vec<i64> = ids.iter().filter_map(|s| s.parse::<i64>().ok()).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let taken: Vec<i64> = ids.iter().copied().take(k.max(1)).collect();
    let storage = state.storage.clone();
    let chunks = store::blocking(move || {
        let mut out: Vec<(arags_storage::sqlite::chunks::Chunk, Option<String>)> =
            Vec::with_capacity(taken.len());
        for id in &taken {
            if as_of_epoch > 0 {
                if let Some(ch) = storage.get_chunk_as_of(*id, as_of_epoch)? {
                    let content = storage.get_chunk_content(ch.id)?.unwrap_or_default();
                    out.push((ch, Some(content)));
                }
            } else if let Some((c, content)) =
                storage.get_chunks_with_content(&[*id])?.into_iter().next()
            {
                out.push((c, content));
            }
        }
        Ok::<_, anyhow::Error>(out)
    })
    .await
    .ok()
    .unwrap_or_default();
    chunks
        .into_iter()
        .map(|(c, content)| SearchResult {
            chunk_id: c.id,
            text: content.unwrap_or_default(),
            score: 1.0,
            file_path: c.file_path,
            start_line: c.line_start as i32,
            end_line: c.line_end as i32,
            epoch: c.epoch,
            created_by: c.created_by.clone().unwrap_or_default(),
            model: c.model.clone().unwrap_or_default(),
            version: c.version,
        })
        .collect()
}

/// Top-K chunk ids from a hybrid search (for the secondary Jaccard check).
pub(crate) async fn top_chunk_ids(
    state: &AppState,
    _project: &str,
    question: &str,
    k: usize,
) -> Vec<String> {
    // Best-effort; if search fails, an empty list yields a failing Jaccard and
    // forces a MISS (safe default).
    match hybrid_search(
        state,
        0,
        &sanitize_fts(question),
        arags_search::SearchTier::Vector,
        k,
    )
    .await
    {
        Ok(results) => results
            .iter()
            .map(|r| r.chunk_id.to_string())
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    }
}
