//! Lazy verify-on-hit grounding (plan 022.8).
//!
//! When enabled, every surfaced map has its claim (`## Conexões` text, via
//! [`arags_core::exploration::claim_text`]) embedded and searched against the
//! CURRENT chunk vectors of its project. Weak or missing evidence forces the
//! map to present as `stale` — catching Cenário B drift that hash anchors
//! cannot see. Cost is one embed + one ANN query per surfaced hit; hits are
//! rare and the feature defaults off.

use crate::state::AppState;
use tracing::{debug, warn};

use super::search::Candidate;

const GROUNDING_TOP_K: usize = 5;

/// Lazy verify-on-hit (plan 022.8): ground the map's claim (`## Conexões`
/// text) against the CURRENT chunk vectors of its project. Weak or missing
/// evidence forces `stale` with a granular reason — the map may describe
/// something the corpus no longer contains, even with intact hash anchors.
/// Cost is one embed + one ANN query per surfaced hit; hits are rare and the
/// feature defaults off.
pub(crate) async fn ground_candidate(state: &AppState, cand: &Candidate) -> Option<Grounding> {
    let vectors = state.vector_store.as_ref()?;
    if !state.config.exploration.verify_on_hit {
        return None;
    }
    let claim = arags_core::exploration::claim_text(&cand.row.body);
    if claim.is_empty() {
        return Some(Grounding::Unsupported);
    }
    let Some(claim_vec) = super::embed_lenient(state, claim.to_string()).await else {
        return None; // embedder unavailable: never downgrade on our own failure
    };
    let buffer_id = u64::try_from(cand.row.buffer_id.unwrap_or(0)).unwrap_or(0);
    match vectors
        .search_similar(&claim_vec, Some(buffer_id), GROUNDING_TOP_K)
        .await
    {
        Ok(matches) => {
            // Chunk space uses L2sq over unit-normalized MiniLM vectors, where
            // `cos = 1 - L2sq / 2` holds exactly; clamp guards degenerate rows.
            let best_raw = matches.first().map_or(f32::INFINITY, |m| m.distance);
            let best = (1.0 - best_raw / 2.0).clamp(0.0, 1.0);
            debug!(
                rowid = cand.row.id,
                best_similarity = best,
                "grounding check"
            );
            if best >= state.config.exploration.grounding_min_similarity {
                Some(Grounding::Supported)
            } else {
                Some(Grounding::Unsupported)
            }
        }
        Err(e) => {
            warn!(error = %e, "grounding search failed; keeping map status");
            None
        }
    }
}

pub(crate) enum Grounding {
    Supported,
    Unsupported,
}
