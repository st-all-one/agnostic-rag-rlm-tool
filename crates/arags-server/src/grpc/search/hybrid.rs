//! Hybrid chunk search plumbing: `buffer_id_for`, `hybrid_search`, and score
//! normalisation shared by the `Search` and `BuildContext` handlers.

use arags_search::{
    Bm25Search, EntitySearch, HybridSearch, SearchOptions, SearchTier as HybridTier,
    SemanticSearch, build_search_results,
};
use tonic::Status;
use tracing::{debug, warn};

use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

/// Map a project reference (UUID or name) to its numeric buffer id.
pub(crate) async fn buffer_id_for(state: &AppState, project: &str) -> Result<Option<i64>, Status> {
    let project_owned = project.to_string();
    let storage = state.storage.clone();
    store::blocking(move || store::buffer_id_for_project(&storage, &project_owned))
        .await
        .map_err(internal)
}

/// Run the unified hybrid search and hydrate results into full chunks.
///
/// Always runs BM25; adds the `entity`/`vector` tiers according to `tier`.
/// When the query is a multi-word natural-language question that returns
/// nothing, a second OR-based BM25 pass recovers relevant chunks.
pub(crate) async fn hybrid_search(
    state: &AppState,
    buffer_id: i64,
    fts_query: &str,
    tier: HybridTier,
    top_k: usize,
) -> anyhow::Result<Vec<arags_search::SearchResult>> {
    let storage = state.storage.clone();
    let bm25 = Bm25Search::new(&storage).map_err(|e| anyhow::anyhow!("bm25 init: {e}"))?;
    let entity = EntitySearch::new(storage.clone()).ok();
    let semantic = state
        .vector_store
        .as_ref()
        .map(|v| SemanticSearch::new(v.clone()));
    // Serving-path decay ([search].decay_lambda > 0) re-weights fused scores
    // by chunk age; 0 keeps the default disabled-at-query-time behaviour.
    let hybrid = {
        let h = HybridSearch::new(bm25, entity, semantic);
        let lambda = state.config.search.decay_lambda;
        if lambda > 0.0 {
            h.with_decay(arags_search::DecayConfig::new(lambda))
        } else {
            h.with_decay(arags_search::DecayConfig::disabled())
        }
    };

    // Embedding inference is synchronous CPU work and would block the async
    // worker, so run it on a blocking task. Falls back to BM25-only when the
    // embed fails. The query embed runs on the **global** rayon pool (not the
    // capped index pool), so a concurrent `arags index` cannot starve it (issue
    // `agnostic-rlm-rs-6690`). We surface contention for observability only.
    if state.index_embed_in_flight() > 0 {
        debug!(
            active_index_embeds = state.index_embed_in_flight(),
            "query embed shares cores with an active index embed; served on global pool"
        );
    }
    let fts_query_owned = fts_query.to_string();
    let embedder = state.embedder.clone();
    let query_vector = tokio::task::spawn_blocking(move || embedder.embed(&fts_query_owned))
        .await
        .ok()
        .and_then(Result::ok);
    let query_vector = query_vector.as_deref();
    let options = SearchOptions {
        tier,
        top_k: top_k * 3,
    };

    let mut fused = hybrid
        .search(fts_query, query_vector, buffer_id, &options, None)
        .await?;

    // Natural-language fix: AND yields nothing on multi-word queries, retry
    // with OR so questions still surface relevant chunks.
    if fused.is_empty() && fts_query.split_whitespace().count() > 1 {
        let or_query = fts_query
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" OR ");
        fused = hybrid
            .search(&or_query, query_vector, buffer_id, &options, None)
            .await?;
    }

    // Serving-path salience decay ([search].decay_lambda > 0): re-weight the
    // fused scores by chunk age and re-rank. Best-effort — an age lookup
    // failure must not kill the search.
    if state.config.search.decay_lambda > 0.0 && !fused.is_empty() {
        let ids: Vec<i64> = fused.iter().map(|r| r.chunk_id).collect();
        let storage = state.storage.clone();
        match store::blocking(move || storage.chunk_ages_hours(&ids)).await {
            Ok(ages) => fused = hybrid.apply_decay(fused, &ages),
            Err(e) => warn!(error = %e, "chunk age lookup failed; skipping decay"),
        }
    }

    let mut results = build_search_results(&state.storage, &fused, None)?;
    normalize_scores(&mut results);
    Ok(results)
}

/// Min-max normalise RRF fusion scores to `[0, 1]` (higher = better) so that
/// `--min-score` thresholds and downstream ranking remain meaningful.
fn normalize_scores(results: &mut [arags_search::SearchResult]) {
    if results.is_empty() {
        return;
    }
    let min = results
        .iter()
        .map(|r| r.score)
        .fold(f32::INFINITY, f32::min);
    let max = results
        .iter()
        .map(|r| r.score)
        .fold(f32::NEG_INFINITY, f32::max);

    if (max - min).abs() < f32::EPSILON {
        for r in results.iter_mut() {
            r.score = 1.0;
        }
        return;
    }
    for r in results.iter_mut() {
        r.score = (r.score - min) / (max - min);
    }
}
