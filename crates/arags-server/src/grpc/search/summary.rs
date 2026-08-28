//! RLM recursive-summary search: lexical + semantic fusion over the dedicated
//! summary vector space, with optional time-travel rehydration.

use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, warn};

use arags_search::hybrid::rrf::rrf_score;

use crate::grpc::error::internal;
use crate::state::AppState;
use crate::store;

/// Time-travel metadata captured for an RLM summary revision: `(summary_text,
/// subject, epoch, created_by, model, version)`.
type RlmAsOf = (String, String, i64, Option<String>, Option<String>, i64);

/// A hydrated RLM summary candidate with its fused score.
#[derive(Debug, Clone)]
pub(crate) struct SummaryCandidate {
    pub node_id: String,
    pub rowid: i64,
    pub level: i64,
    pub subject: String,
    pub text: String,
    pub score: f32,
    pub epoch: i64,
    pub created_by: Option<String>,
    pub model: Option<String>,
    pub version: i64,
}

/// Search the RLM recursive summary dataset. Lexical candidates come from
/// `rlm_fts`; semantic candidates from the dedicated summary vector space.
/// The two rankings are fused with Reciprocal Rank Fusion (same family as the
/// chunk hybrid tiers) and min-max normalised to `[0, 1]`.
pub(crate) async fn summary_search(
    state: &AppState,
    buffer_id: i64,
    project: &str,
    fts_query: &str,
    top_k: usize,
    as_of_epoch: i64,
) -> anyhow::Result<Vec<SummaryCandidate>> {
    const RRF_K: f32 = 60.0;
    let start = Instant::now();
    let storage = state.storage.clone();

    // Lexical pass (blocking SQLite work): ranked list of rowids.
    let query_owned = fts_query.to_string();
    let lexical: Vec<(i64, arags_storage::sqlite::rlm::RlmNode)> =
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut nodes = storage.search_rlm_fts(buffer_id, &query_owned, top_k)?;
            if nodes.is_empty() && query_owned.split_whitespace().count() > 1 {
                // Natural-language fix: AND yields nothing, retry with OR.
                let or_query = query_owned
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" OR ");
                nodes = storage.search_rlm_fts(buffer_id, &or_query, top_k)?;
            }
            Ok(nodes.into_iter().map(|n| (n.id, n)).collect())
        })
        .await
        .map_err(internal)??;

    // Semantic pass over the dedicated summary vector space: ranked list
    // ordered by cosine similarity (approved + scoped hydration only).
    let mut semantic: Vec<(i64, arags_storage::sqlite::rlm::RlmNode)> = Vec::new();
    if let Some(vectors) = state.rlm_vector_store.as_ref() {
        let embedder = state.embedder.clone();
        let q = fts_query.to_string();
        // Query embed runs on the global rayon pool; the capped index pool keeps
        // it from being starved during a concurrent `arags index` (issue
        // `agnostic-rag-rlm-tool-6690`).
        if state.index_embed_in_flight() > 0 {
            debug!(
                active_index_embeds = state.index_embed_in_flight(),
                "rlm summary query embed contends with active index embed; served on global pool"
            );
        }
        let query_vector = tokio::task::spawn_blocking(move || embedder.embed(&q))
            .await
            .ok()
            .and_then(Result::ok);
        if let Some(vec) = query_vector {
            let mut neighbors = vectors
                .search(&vec, top_k)
                .map_err(|e| anyhow::anyhow!(e))?;
            neighbors.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
            let ids: Vec<u64> = neighbors.iter().map(|n| n.id).collect();
            let storage = state.storage.clone();
            let approved = store::blocking(move || storage.get_approved_rlm_nodes(&ids, buffer_id))
                .await
                .map_err(internal)?;
            let by_id: HashMap<i64, _> = approved.into_iter().map(|n| (n.id, n)).collect();
            for nb in neighbors {
                #[allow(clippy::cast_possible_wrap)] // rowids fit i64
                if let Ok(rowid) = i64::try_from(nb.id) {
                    if let Some(node) = by_id.get(&rowid) {
                        semantic.push((rowid, node.clone()));
                    }
                }
            }
        }
    }

    // RRF fusion over the two rankings; hydrate from whichever list carries
    // the node (lexical rows are complete; semantic rows were hydrated too).
    let mut scores: HashMap<i64, f32> = HashMap::with_capacity(top_k * 2);
    for (rank, (rowid, _)) in lexical.iter().enumerate() {
        *scores.entry(*rowid).or_insert(0.0) += rrf_score(rank, RRF_K);
    }
    for (rank, (rowid, _)) in semantic.iter().enumerate() {
        *scores.entry(*rowid).or_insert(0.0) += rrf_score(rank, RRF_K);
    }
    let node_by_id: HashMap<i64, &arags_storage::sqlite::rlm::RlmNode> = lexical
        .iter()
        .chain(semantic.iter())
        .map(|(id, n)| (*id, n))
        .collect();

    let mut fused: Vec<SummaryCandidate> = scores
        .into_iter()
        .filter_map(|(rowid, score)| {
            node_by_id.get(&rowid).map(|n| SummaryCandidate {
                node_id: n.node_id.clone(),
                rowid,
                level: n.level,
                subject: n.subject.clone(),
                text: n.summary_text.clone(),
                score,
                epoch: n.epoch,
                created_by: n.created_by.clone(),
                model: n.model.clone(),
                version: n.version,
            })
        })
        .collect();

    // Time-travel (plan 021): for each summary candidate, serve the revision of
    // its `(project, level, subject)` node that was active at `as_of_epoch`.
    if as_of_epoch > 0 {
        let storage = state.storage.clone();
        let project_owned = project.to_string();
        let keys: Vec<(i64, i64, String)> = fused
            .iter()
            .map(|s| (s.rowid, s.level, s.subject.clone()))
            .collect();
        match store::blocking(move || {
            let mut revised: HashMap<i64, RlmAsOf> = HashMap::with_capacity(keys.len());
            for (rowid, level, subject) in keys {
                if let Some(node) =
                    storage.get_rlm_node_as_of(&project_owned, level, &subject, as_of_epoch)?
                {
                    revised.insert(
                        rowid,
                        (
                            node.summary_text.clone(),
                            node.subject.clone(),
                            node.epoch,
                            node.created_by.clone(),
                            node.model.clone(),
                            node.version,
                        ),
                    );
                }
            }
            Ok::<_, anyhow::Error>(revised)
        })
        .await
        {
            Ok(revised) => {
                for s in &mut fused {
                    if let Some((text, subject, epoch, created_by, model, version)) =
                        revised.get(&s.rowid)
                    {
                        s.text = text.clone();
                        s.subject = subject.clone();
                        s.epoch = *epoch;
                        s.created_by = created_by.clone();
                        s.model = model.clone();
                        s.version = *version;
                    }
                }
            }
            Err(e) => warn!(error = %e, "summary as_of rewrite failed; serving live"),
        }
    }

    // Deterministic order: score desc, then rowid asc (RRF ties).
    fused.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.rowid.cmp(&b.rowid))
    });
    normalize_summaries(&mut fused);
    fused.truncate(top_k);
    debug!(
        elapsed_ms = start.elapsed().as_millis(),
        lexical = lexical.len(),
        semantic = semantic.len(),
        fused = fused.len(),
        "summary_search completed"
    );
    Ok(fused)
}

/// Min-max normalise RRF-fused summary scores to `[0, 1]`.
fn normalize_summaries(summaries: &mut [SummaryCandidate]) {
    if summaries.len() < 2 {
        for s in summaries.iter_mut() {
            s.score = 1.0;
        }
        return;
    }
    let min = summaries
        .iter()
        .map(|s| s.score)
        .fold(f32::INFINITY, f32::min);
    let max = summaries
        .iter()
        .map(|s| s.score)
        .fold(f32::NEG_INFINITY, f32::max);
    if (max - min).abs() < f32::EPSILON {
        for s in summaries.iter_mut() {
            s.score = 1.0;
        }
        return;
    }
    for s in summaries.iter_mut() {
        s.score = (s.score - min) / (max - min);
    }
}
