//! RLM cosine-quorum decision engine (issue `agnostic-rlm-rs-6d97`, plan
//! `pl-84c3` step 2).
//!
//! A subject fanned out to `n` volunteer slots (each staged as a `candidate`
//! submission in `submissions`) is decided once `n` candidates are gathered:
//!
//! 1. Embed every pending candidate (batch, on the capped index-embed rayon
//!    pool — same pattern as [`crate::reconcile`]).
//! 2. Compute pairwise cosine similarity and find the largest **clique** where
//!    every pairwise similarity is `>= quorum_sim_threshold` (the agreeing set).
//! 3. If such a quorum exists, fuse the agreeing set per [`FusionStrategy`]
//!    (consensus centroid / embedding average / longest) and publish the fused
//!    summary as the live, approved RLM node. The winning candidates are
//!    `accept`ed; the dissenting ones are `reject`ed (their similarity to the
//!    consensus is recorded and a strike is filed against their author).
//! 4. If no two candidates agree, every candidate is `reject`ed and a strike is
//!    filed against each author — the subject is left for human review / later
//!    re-fan-out.
//!
//! All SQLite access is scoped inside [`crate::store::blocking`]; embedding runs
//! outside the lock on the capped pool. The decision is idempotent: a subject
//! that already has an `accepted` submission is never re-decided.

use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{info, instrument, warn};

use arags_embedding::embedder::Embedding;
use arags_search::qa_cache::cosine_similarity;
use arags_storage::sqlite::rlm::rlm_job_key;

use crate::config::{FusionStrategy, QuorumConfig};
use crate::state::AppState;
use crate::store;

/// Outcome of a quorum decision for one RLM subject.
#[derive(Debug, Clone, PartialEq)]
pub enum QuorumDecision {
    /// Fewer than `n` candidates gathered yet — defer.
    Pending,
    /// A consensus was reached and published.
    Accepted {
        /// Fused summary text published as the live RLM node.
        fused_text: String,
        /// Row ids of the submissions that formed the consensus.
        accepted_submission_ids: Vec<i64>,
        /// Row ids of the dissenting submissions (rejected + struck).
        rejected_submission_ids: Vec<i64>,
    },
    /// No two candidates agreed; all rejected, subject left for review.
    Rejected {
        /// Row ids of every rejected submission.
        rejected_submission_ids: Vec<i64>,
    },
}

/// Decide (or report the status of) the RLM quorum for `(project, level,
/// subject)`.
///
/// Idempotent: if the subject already has an `accepted` submission the prior
/// decision is returned without re-running; if candidates are still being
/// gathered it returns [`QuorumDecision::Pending`].
///
/// # Errors
///
/// Returns an error if storage, embedding or the node publish fails (per-row
/// failures are logged, not fatal to the whole decision).
#[instrument(skip_all, fields(phase = "rlm_quorum_decision", subject))]
pub async fn decide_rlm_quorum(
    state: &AppState,
    project: &str,
    level: i64,
    subject: &str,
) -> Result<QuorumDecision> {
    let start = Instant::now();
    let subject_key = rlm_job_key(project, level, subject);
    let n = state.config.quorum.n.max(1);
    let threshold = state.config.quorum.quorum_sim_threshold;

    // Idempotency / gating: gather pending candidates.
    let pending = store::blocking({
        let storage = state.storage.clone();
        let (p, t, k) = (
            project.to_string(),
            "rlm_node".to_string(),
            subject_key.clone(),
        );
        move || storage.list_pending(&p, &t, &k)
    })
    .await
    .context("list pending rlm submissions")?;

    if pending.is_empty() {
        // Already decided? Return the prior decision for idempotency.
        let accepted = store::blocking({
            let storage = state.storage.clone();
            let (p, t, k) = (
                project.to_string(),
                "rlm_node".to_string(),
                subject_key.clone(),
            );
            move || storage.list_accepted(&p, &t, &k)
        })
        .await
        .context("list accepted rlm submissions")?;
        if let Some(win) = accepted.first() {
            info!(
                phase = "rlm_quorum_decision",
                subject,
                already = "accepted",
                "quorum already decided; idempotent no-op"
            );
            return Ok(QuorumDecision::Accepted {
                fused_text: win.candidate_text.clone(),
                accepted_submission_ids: vec![win.id],
                rejected_submission_ids: Vec::new(),
            });
        }
        return Ok(QuorumDecision::Rejected {
            rejected_submission_ids: Vec::new(),
        });
    }

    if pending.len() < n {
        info!(
            phase = "rlm_quorum_decision",
            subject,
            gathered = pending.len(),
            needed = n,
            "quorum awaiting more candidates"
        );
        return Ok(QuorumDecision::Pending);
    }

    // Embed every candidate (batch on the capped pool).
    let texts: Vec<String> = pending.iter().map(|s| s.candidate_text.clone()).collect();
    let vectors = match embed_candidates(state, &texts).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, subject, "rlm quorum embedding failed; deferring");
            return Ok(QuorumDecision::Pending);
        }
    };

    // Pairwise cosine similarity matrix.
    let sims: Vec<Vec<f32>> = (0..vectors.len())
        .map(|i| {
            (0..vectors.len())
                .map(|j| cosine_similarity(&vectors[i], &vectors[j]))
                .collect()
        })
        .collect();

    // Largest agreeing clique (every pairwise >= threshold). A single
    // candidate is only a quorum when n == 1.
    let agreeing = if n == 1 {
        vec![0]
    } else {
        best_agreeing_clique(&sims, threshold)
    };

    let elapsed_ms = start.elapsed().as_millis() as u64;

    if agreeing.is_empty() {
        // No consensus: reject every candidate and strike each author.
        let mut rejected_ids = Vec::new();
        for sub in &pending {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let id = sub.id;
                let by = sub.candidate_by.clone();
                move || -> anyhow::Result<()> {
                    storage.reject_submission(id, "quorum", None)?;
                    storage.record_strike(&by)?;
                    Ok(())
                }
            })
            .await
            {
                warn!(error = %e, subject, "rlm quorum reject/strike failed");
            } else {
                rejected_ids.push(sub.id);
            }
        }
        info!(
            phase = "rlm_quorum_decision",
            subject,
            elapsed_ms,
            rejected = rejected_ids.len(),
            "quorum rejected: no consensus"
        );
        return Ok(QuorumDecision::Rejected {
            rejected_submission_ids: rejected_ids,
        });
    }

    // Fuse the agreeing set into the accepted summary text.
    let fused = fuse_agreement(&state.config.quorum, &agreeing, &pending, &vectors);

    // Publish the node (approved — the quorum is the quality gate).
    let (rowid, node_id) = store::blocking({
        let storage = state.storage.clone();
        let (p, s, f) = (project.to_string(), subject.to_string(), fused.clone());
        move || {
            storage.publish_rlm_node_for_subject(&p, level, &s, &f, &[], None, None, None, "quorum")
        }
    })
    .await
    .context("publish quorum rlm node")?;

    // Consensus embedding = mean of agreeing vectors.
    let fused_vec = centroid(&agreeing, &vectors);

    let mut accepted_ids = Vec::new();
    let mut rejected_ids = Vec::new();
    for (i, sub) in pending.iter().enumerate() {
        let sim = cosine_similarity(&fused_vec, &vectors[i]);
        if agreeing.contains(&i) {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let id = sub.id;
                move || storage.accept_submission(id, "quorum", Some(f64::from(sim)))
            })
            .await
            {
                warn!(error = %e, subject, "rlm quorum accept failed");
            } else {
                accepted_ids.push(sub.id);
            }
        } else {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let id = sub.id;
                let by = sub.candidate_by.clone();
                move || -> anyhow::Result<()> {
                    storage.reject_submission(id, "quorum", Some(f64::from(sim)))?;
                    storage.record_strike(&by)?;
                    Ok(())
                }
            })
            .await
            {
                warn!(error = %e, subject, "rlm quorum reject/strike failed");
            } else {
                rejected_ids.push(sub.id);
            }
        }
    }

    // Hydrate the new node's vector for semantic search.
    if let Some(vs) = &state.rlm_vector_store {
        let text = format!("{subject}\n{fused}");
        match tokio::task::spawn_blocking({
            let embedder = state.embedder.clone();
            let text = text.clone();
            move || embedder.embed(&text)
        })
        .await
        {
            Ok(Ok(vec)) => {
                #[allow(clippy::cast_possible_truncation)] // rowids fit u64
                let key = u64::try_from(rowid).unwrap_or(u64::MAX);
                if let Err(e) = vs.insert(key, &vec) {
                    warn!(error = %e, node_id = %node_id, "rlm quorum vector insert failed");
                }
            }
            Ok(Err(e)) => warn!(error = %e, node_id = %node_id, "rlm quorum embed failed"),
            Err(e) => warn!(error = %e, node_id = %node_id, "rlm quorum embed task panicked"),
        }
    }

    // Cascade to the parent level (mirrors the single-volunteer completion path).
    if state.config.rlm.enabled && level < 3 {
        let tolerance = if level == 1 {
            state.config.rlm.l2_tolerance
        } else {
            state.config.rlm.l3_tolerance
        };
        let buffer_id = store::blocking({
            let storage = state.storage.clone();
            let (p, s) = (project.to_string(), subject.to_string());
            move || {
                storage
                    .get_rlm_node_by_subject(&p, level, &s)
                    .map(|nd| nd.and_then(|x| x.buffer_id))
            }
        })
        .await
        .ok()
        .flatten();
        if let Some(buf) = buffer_id {
            if let Err(e) = store::blocking({
                let storage = state.storage.clone();
                let qn = state.config.quorum.n.max(1);
                let (p, s) = (project.to_string(), subject.to_string());
                move || crate::store::rlm::cascade_rlm(&storage, buf, &p, level, &s, tolerance, qn)
            })
            .await
            {
                warn!(error = %e, subject, "rlm quorum cascade failed");
            }
        }
    }

    info!(
        phase = "rlm_quorum_decision",
        subject,
        elapsed_ms,
        accepted = accepted_ids.len(),
        rejected = rejected_ids.len(),
        node_id = %node_id,
        "quorum accepted: consensus published"
    );

    Ok(QuorumDecision::Accepted {
        fused_text: fused,
        accepted_submission_ids: accepted_ids,
        rejected_submission_ids: rejected_ids,
    })
}

/// Embed a batch of candidate texts on the capped index-embed rayon pool.
///
/// # Errors
///
/// Returns an error if the embedding task panics or the embedder fails.
async fn embed_candidates(state: &AppState, texts: &[String]) -> Result<Vec<Embedding>> {
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
    .context("rlm quorum embedding task panicked")?
    .map_err(|e| anyhow::anyhow!("rlm quorum embedding failed: {e}"))
}

/// Find the largest subset of candidates where every pairwise cosine similarity
/// is `>= threshold`. Ties on size are broken by the higher minimum in-clique
/// similarity (denser agreement wins).
fn best_agreeing_clique(sims: &[Vec<f32>], threshold: f64) -> Vec<usize> {
    let n = sims.len();
    if n < 2 {
        return Vec::new();
    }
    let mut best: Vec<usize> = Vec::new();
    let mut best_size = 0usize;
    let mut best_min = f64::NEG_INFINITY;
    for mask in 1..(1u32 << n) {
        let members: Vec<usize> = (0..n).filter(|i| mask & (1u32 << i) != 0).collect();
        if members.len() < 2 {
            continue;
        }
        let mut ok = true;
        let mut min_sim = f64::INFINITY;
        for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                let s = f64::from(sims[members[a]][members[b]]);
                if s < threshold {
                    ok = false;
                }
                min_sim = min_sim.min(s);
            }
        }
        if ok && (members.len() > best_size || (members.len() == best_size && min_sim > best_min)) {
            best_size = members.len();
            best_min = min_sim;
            best = members;
        }
    }
    best
}

/// Mean vector of the agreeing candidates.
fn centroid(agreeing: &[usize], vectors: &[Embedding]) -> Embedding {
    let dims = vectors.first().map_or(0, Vec::len);
    let mut acc = vec![0.0_f32; dims];
    for &i in agreeing {
        for (d, v) in acc.iter_mut().enumerate() {
            *v += vectors[i][d];
        }
    }
    let denom = agreeing.len().max(1) as f32;
    for v in &mut acc {
        *v /= denom;
    }
    acc
}

/// Fuse the agreeing candidates into the accepted summary text per `strategy`.
fn fuse_agreement(
    cfg: &QuorumConfig,
    agreeing: &[usize],
    pending: &[arags_storage::sqlite::submissions::Submission],
    vectors: &[Embedding],
) -> String {
    match cfg.fusion_strategy {
        FusionStrategy::Longest => agreeing
            .iter()
            .map(|&i| pending[i].candidate_text.as_str())
            .max_by_key(|t| t.len())
            .unwrap_or("")
            .to_string(),
        FusionStrategy::Consensus | FusionStrategy::Average => {
            let center = centroid(agreeing, vectors);
            agreeing
                .iter()
                .map(|&i| (i, cosine_similarity(&center, &vectors[i])))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(i, _)| pending[i].candidate_text.clone())
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests;
