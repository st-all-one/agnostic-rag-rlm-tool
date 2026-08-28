//! RLM cosine-quorum decision engine (issue `agnostic-rag-rlm-tool-6d97`, plan
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

use std::collections::HashMap;
use std::time::Instant;
use tracing::{info, instrument, warn};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use rusqlite::params;

use arags_embedding::embedder::Embedding;
use arags_search::qa_cache::cosine_similarity;
use arags_storage::sqlite::rlm::{PRIORITY_RETRY, rlm_job_key};

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

    // Byzantine bound (issue `agnostic-rag-rlm-tool-64af`): tolerate up to
    // `f = floor((n - 1) / 3)` malicious volunteers; a valid quorum needs at
    // least `2f + 1` mutually-agreeing candidates. With `n >= 3f + 1` and at
    // most `f` byzantine, an honest majority yields a unique accepted value.
    let f = (n.saturating_sub(1)) / 3;
    let min_clique = 2usize
        .checked_mul(f)
        .map_or(usize::MAX, |v| v.saturating_add(1));

    // Largest agreeing clique (every pairwise >= threshold). A single
    // candidate is only a quorum when n == 1.
    let agreeing = if n == 1 {
        vec![0]
    } else {
        best_agreeing_clique(&sims, threshold)
    };

    let elapsed_ms = start.elapsed().as_millis() as u64;

    // No quorum if the agreeing set is empty OR smaller than the 2f+1 bound:
    // reject every candidate and strike each author.
    if agreeing.is_empty() || agreeing.len() < min_clique {
        // No consensus: reject every candidate and strike each author.
        let mut rejected_ids = Vec::new();
        let mut divergers: Vec<String> = Vec::new();
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
                if !divergers.contains(&sub.candidate_by) {
                    divergers.push(sub.candidate_by.clone());
                }
            }
        }
        info!(
            phase = "rlm_quorum_decision",
            subject,
            elapsed_ms,
            rejected = rejected_ids.len(),
            "quorum rejected: no consensus"
        );

        // Total divergence: auto re-fan-out to a NEW generation group, excluding
        // the volunteers that just diverged so the same answers are not repeated.
        // Capped at `strikes_limit` rounds (preventing infinite loops); when
        // exhausted the subject is left for human review.
        reassign_rlm_on_divergence(state, project, level, subject, &divergers, n, elapsed_ms).await;

        return Ok(QuorumDecision::Rejected {
            rejected_submission_ids: rejected_ids,
        });
    }

    // Trust-weighted fusion (issue `agnostic-rag-rlm-tool-64af`): weight the published
    // choice by each agreeing candidate's `volunteer_trust.trust_score` so a
    // higher-trust volunteer's answer is preferred when several agree. All DB
    // reads stay inside `store::blocking`.
    let trust_scores: HashMap<String, f64> = store::blocking({
        let storage = state.storage.clone();
        let vols: Vec<String> = pending.iter().map(|s| s.candidate_by.clone()).collect();
        move || -> anyhow::Result<HashMap<String, f64>> {
            let mut m = HashMap::new();
            for v in &vols {
                let (_, t) = storage.read_trust(v)?;
                m.insert(v.clone(), t);
            }
            Ok(m)
        }
    })
    .await
    .context("read volunteer trust for quorum fusion")?;

    // Choose the published candidate index: highest trust, tie-broken by closeness
    // to the consensus centroid. With `n >= 3f + 1` and at most `f` byzantine
    // volunteers, an honest majority guarantees a unique accepted value.
    let published_idx = choose_published(
        &state.config.quorum,
        &agreeing,
        &pending,
        &vectors,
        &trust_scores,
    );
    let fused = pending[published_idx].candidate_text.clone();
    let published_by = pending[published_idx].candidate_by.clone();

    // Publish the node (approved — the quorum is the quality gate), attributed to
    // the highest-trust agreeing volunteer.
    let (rowid, node_id) = store::blocking({
        let storage = state.storage.clone();
        let (p, s, f, by) = (
            project.to_string(),
            subject.to_string(),
            fused.clone(),
            published_by.clone(),
        );
        move || {
            storage.publish_rlm_node_for_subject(
                &p,
                level,
                &s,
                &f,
                &[],
                Some(&by),
                None,
                None,
                "quorum",
            )
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
                let by = sub.candidate_by.clone();
                move || -> anyhow::Result<()> {
                    storage.accept_submission(id, "quorum", Some(f64::from(sim)))?;
                    // An accepted candidate lifts its author's trust.
                    storage.bump_trust_on_accept(&by)?;
                    Ok(())
                }
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

/// After a total-divergence rejection, re-fan the subject out to a fresh
/// generation group of `n` slots while excluding the volunteers that just
/// diverged (issue `agnostic-rag-rlm-tool-f486`). Re-fan-outs are capped at
/// `quorum.strikes_limit` rounds: once the subject's generation reaches that
/// ceiling (or every known volunteer is banned/excluded) the subject is left
/// for human review and a `phase = "rlm_quorum_reassign", status = "exhausted"`
/// event is emitted with `elapsed_ms`. All DB work runs inside
/// `store::blocking`.
#[instrument(skip_all, fields(phase = "rlm_quorum_reassign", subject))]
async fn reassign_rlm_on_divergence(
    state: &AppState,
    project: &str,
    level: i64,
    subject: &str,
    divergers: &[String],
    n: usize,
    elapsed_ms: u64,
) {
    let strikes_limit = state.config.quorum.strikes_limit;
    let p = project.to_string();
    let s = subject.to_string();

    // Current generation of this subject; used both to cap rounds and to detect
    // whether any non-banned volunteer remains to do the work.
    let (current_generation, known_volunteers, banned_volunteers): (i64, Vec<String>, Vec<String>) = {
        let storage = state.storage.clone();
        let (pp, ss) = (p.clone(), s.clone());
        store::blocking(move || {
            let max_gen: i64 = storage
                .connection()
                .context("acquire connection")?
                .execute(|c| {
                    c.query_row(
                        "SELECT COALESCE(MAX(generation), 0) FROM rlm_jobs \
                         WHERE project = ?1 AND level = ?2 AND subject = ?3",
                        params![pp, level, ss],
                        |r| r.get(0),
                    )
                    .context("read rlm_job generation")
                })?;
            // Known volunteer roster (from volunteer_trust) and how many are
            // already banned, to decide whether reassignment is still possible.
            let rows: Vec<(String, i64)> = storage
                .connection()
                .context("acquire connection")?
                .execute(|c| {
                    let mut stmt = c
                        .prepare("SELECT username, strikes FROM volunteer_trust")
                        .context("prepare volunteer roster")?;
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
                        .context("query volunteer roster")?;
                    let mut out = Vec::new();
                    for r in rows {
                        out.push(r.context("map volunteer roster")?);
                    }
                    Ok(out)
                })?;
            let known: Vec<String> = rows.iter().map(|(u, _)| u.clone()).collect();
            let banned: Vec<String> = rows
                .into_iter()
                .filter(|(_, st)| {
                    #[allow(clippy::cast_sign_loss)]
                    {
                        (*st) as u32 >= strikes_limit
                    }
                })
                .map(|(u, _)| u)
                .collect();
            Ok((max_gen, known, banned))
        })
        .await
        .unwrap_or((0, Vec::new(), Vec::new()))
    };

    // Cap 1: generation ceiling reached -> no further re-fan-outs.
    #[allow(clippy::cast_possible_truncation)]
    if current_generation as u32 >= strikes_limit {
        info!(
            phase = "rlm_quorum_reassign",
            status = "exhausted",
            subject,
            elapsed_ms,
            generation = current_generation,
            strikes_limit,
            "rlm quorum re-fan-out capped: leaving for human review"
        );
        return;
    }

    // Cap 2: no non-banned, non-diverging volunteer remains -> human review.
    let available: usize = known_volunteers
        .iter()
        .filter(|v| !banned_volunteers.contains(v) && !divergers.contains(v))
        .count();
    if !divergers.is_empty() && known_volunteers.len() >= n && available == 0 {
        info!(
            phase = "rlm_quorum_reassign",
            status = "exhausted",
            subject,
            elapsed_ms,
            available,
            "rlm quorum re-fan-out exhausted: no non-banned volunteers"
        );
        return;
    }

    // Reuse the subject's last job payload/buffer so the new slots carry the
    // same inputs; fall back to empty if no prior job exists.
    let (buffer_id, payload) = {
        let storage = state.storage.clone();
        let (pp, ss) = (p.clone(), s.clone());
        store::blocking(move || {
            let conn = storage.connection().context("acquire connection")?;
            let row: Option<(Option<i64>, String)> = conn.execute(|c| {
                c.query_row(
                    "SELECT buffer_id, payload FROM rlm_jobs \
                         WHERE project = ?1 AND level = ?2 AND subject = ?3 \
                         ORDER BY id DESC LIMIT 1",
                    params![pp, level, ss],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .context("read rlm_job payload")
            })?;
            Ok(match row {
                Some((b, pld)) => (b, pld),
                None => (None, "{}".to_string()),
            })
        })
        .await
        .unwrap_or((None, "{}".to_string()))
    };

    let job = arags_storage::sqlite::rlm::NewRlmJob {
        buffer_id,
        project: p.clone(),
        level,
        subject: s.clone(),
        payload,
        priority: PRIORITY_RETRY,
        quorum_slots: n,
    };
    match store::blocking({
        let storage = state.storage.clone();
        let exclude = divergers.to_vec();
        move || storage.enqueue_rlm_job(&job, &exclude)
    })
    .await
    {
        Ok((first_id, new_generation, _)) => info!(
            phase = "rlm_quorum_reassign",
            status = "reassigned",
            subject,
            elapsed_ms,
            new_generation,
            new_job_id = first_id,
            excluded = divergers.len(),
            "rlm quorum re-fanned out to a new generation group excluding divergers"
        ),
        Err(e) => warn!(
            error = %e,
            subject,
            "rlm quorum re-fan-out enqueue failed"
        ),
    }
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

/// Pick which agreeing candidate becomes the published RLM node.
///
/// Ranks the agreeing members by a trust-weighted score — `trust_score` is the
/// primary bias (the BFT guarantee of `agnostic-rag-rlm-tool-64af` prefers reputable
/// volunteers), with the similarity to the consensus centroid (or text length
/// for the `Longest` strategy) as the base. Returns the index into `pending`.
///
/// With `n >= 3f + 1` and at most `f` byzantine volunteers, the honest majority
/// forms a unique agreeing set, so this choice is well-defined and stable.
fn choose_published(
    cfg: &QuorumConfig,
    agreeing: &[usize],
    pending: &[arags_storage::sqlite::submissions::Submission],
    vectors: &[Embedding],
    trust: &HashMap<String, f64>,
) -> usize {
    if agreeing.is_empty() {
        return 0;
    }
    let center = centroid(agreeing, vectors);
    let mut best = agreeing[0];
    let mut best_score = f64::NEG_INFINITY;
    for &i in agreeing {
        let base = match cfg.fusion_strategy {
            FusionStrategy::Longest => {
                #[allow(clippy::cast_possible_truncation)]
                {
                    pending[i].candidate_text.len() as f64
                }
            }
            FusionStrategy::Consensus | FusionStrategy::Average => {
                f64::from(cosine_similarity(&center, &vectors[i]))
            }
        };
        // Trust-weight: a higher `trust_score` biases the published answer toward
        // reputable volunteers (clamped so a trust of 0 still contributes half).
        let t = trust.get(&pending[i].candidate_by).copied().unwrap_or(1.0);
        let score = base * (0.5 + 0.5 * t.max(0.0));
        if score > best_score {
            best_score = score;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests;
