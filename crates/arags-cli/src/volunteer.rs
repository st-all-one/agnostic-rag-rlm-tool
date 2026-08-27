//! RLM volunteer worker (`arags volunteer`).
//!
//! Claims summary jobs from the server queue and synthesizes them with the
//! user's local LLM (llama 3.2 by convention). The lease is client-
//! configurable in `~/.arags/arags.toml [volunteer]` (default 500s for every
//! level); a claimed work unit is exclusively ours until the lease expires.
//! Cancellation is cooperative: if the server resets the job's generation
//! while we process, our submission is rejected and we simply try again.

use std::sync::Arc;
use std::time::Duration;

use std::fmt::Write as _;

use anyhow::{Context, Result};
use arags_core::rlm::{DEFAULT_RLM_LEASE_MS, RlmJobPayload};
use arags_llm::trait_llm::LlmBackend;
use arags_llm::types::{CompletionRequest, Message};
use arags_proto::proto::CompleteRlmJobRequest;
use parking_lot::Mutex;

use crate::auth_client;
use crate::backend::resolve_backend;
use crate::user_config::{EffectiveUserConfig, VolunteerConfig};

/// Prompt templates per level ("arlm-v1"). Kept deliberately small so local
/// models like llama 3.2 can follow them reliably; each output feeds the next
/// level verbatim, so structure beats prose.
const SYSTEM_L1: &str = "You summarize source-code files for a retrieval index. \
Output 5-10 dense bullet lines covering: purpose of the file, key types/functions \
with one-line roles, notable patterns/idioms, and dependencies. No preamble, no markdown headers.";
const SYSTEM_L2: &str = "You synthesize module summaries from per-file summaries. \
Output 5-8 dense bullet lines covering: what this module/theme provides, its main \
abstractions, internal organization, and how files relate. No preamble, no headers.";
const SYSTEM_L3: &str = "You write a project overview from module summaries. \
Output: one paragraph stating what the project is and its architecture style, then \
5-8 bullet lines listing the modules/themes and their responsibilities, then any \
cross-cutting patterns. No headers.";

/// Run the volunteer loop. With `once`, processes at most one job then exits
/// (used by tests and smoke runs).
///
/// # Errors
///
/// Returns an error if config/backend/connection are unusable. Job-level
/// failures are logged and retried inside the loop instead.
pub fn run(rt: &tokio::runtime::Runtime, cfg: &EffectiveUserConfig, once: bool) -> Result<()> {
    let vol = cfg.volunteer.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "volunteer mode not configured: add [volunteer] enabled = true to ~/.arags/arags.toml"
        )
    })?;
    if !vol.enabled {
        anyhow::bail!("volunteer disabled ([volunteer] enabled = false)");
    }

    let backend = resolve_backend(
        cfg.llm_config(),
        vol.backend.as_deref(),
        vol.model.as_deref(),
    )
    .context("failed to resolve LLM backend for volunteer")?;
    // The request must carry the concrete model name (backends send it
    // verbatim to the provider; e.g. Ollama 404s on an empty model).
    let model_name = vol
        .model
        .clone()
        .or_else(|| {
            cfg.llm_config()
                .and_then(|l| l.backends.first().and_then(|b| b.model.clone()))
        })
        .unwrap_or_else(|| "llama3.2".to_string());

    let client_config = crate::client::ClientConfig {
        addr: cfg.server_addr(),
        tls_ca: cfg.server.tls_ca.clone(),
        tls_cert: cfg.server.tls_cert.clone(),
        tls_key: cfg.server.tls_key.clone(),
    };
    let auth = cfg.auth().ok_or_else(|| {
        anyhow::anyhow!("volunteer requires [auth] refresh_token in ~/.arags/arags.toml")
    })?;
    let (mut client, session_token) = auth_client::connect(rt, &client_config, auth)?;

    tracing::info!(
        lease_secs = vol.lease_secs,
        max_level = vol.max_level,
        once,
        "volunteer started"
    );

    loop {
        let claimed = rt.block_on(claim(&mut client, vol));
        match claimed {
            Ok(None) => {
                if once {
                    tracing::info!("no jobs available");
                    return Ok(());
                }
                std::thread::sleep(Duration::from_secs(vol.poll_secs.max(1)));
            }
            Ok(Some(job)) => {
                if let Err(e) = process(
                    rt,
                    &mut client,
                    &backend,
                    &model_name,
                    vol,
                    &session_token,
                    job,
                    once,
                ) {
                    // `once` errors bubble up; loop errors are logged+retried.
                    if once {
                        return Err(e);
                    }
                    tracing::warn!(error = %e, "job processing failed; continuing");
                    std::thread::sleep(Duration::from_secs(vol.poll_secs.max(1)));
                } else if once {
                    return Ok(());
                }
            }
            Err(e) => {
                if once {
                    return Err(e);
                }
                tracing::warn!(error = %e, "claim failed; retrying");
                std::thread::sleep(Duration::from_secs(vol.poll_secs.max(1)));
            }
        }
    }
}

/// A claimed job ready for local synthesis.
struct ClaimedJob {
    id: i64,
    project: String,
    level: i32,
    subject: String,
    payload: String,
    generation: i64,
}

async fn claim(
    client: &mut auth_client::AragsClient,
    vol: &VolunteerConfig,
) -> Result<Option<ClaimedJob>> {
    use arags_proto::proto::ClaimRlmJobRequest;
    let resp = client
        .claim_rlm_job(ClaimRlmJobRequest {
            lease_ms: i64::try_from(vol.lease_secs.saturating_mul(1000))
                .unwrap_or(DEFAULT_RLM_LEASE_MS),
            max_level: i32::try_from(vol.max_level).unwrap_or(3),
        })
        .await
        .context("ClaimRlmJob RPC failed")?
        .into_inner();
    if !resp.available {
        return Ok(None);
    }
    Ok(Some(ClaimedJob {
        id: resp.job_id,
        project: resp.project,
        level: resp.level,
        subject: resp.subject,
        payload: resp.payload,
        generation: resp.generation,
    }))
}

fn system_prompt_for(level: i32) -> &'static str {
    match level {
        1 => SYSTEM_L1,
        2 => SYSTEM_L2,
        _ => SYSTEM_L3,
    }
}

fn build_request(
    level: i32,
    subject: &str,
    payload: &RlmJobPayload,
    max_tokens: u32,
) -> CompletionRequest {
    use arags_llm::types::Role;
    let mut body = format!(
        "Subject: {subject}\nKind: {}\n\nInputs:\n",
        payload.subject_kind
    );
    for (i, text) in payload.texts.iter().enumerate() {
        let _ = writeln!(body, "\n--- input {} ---\n{text}\n", i + 1);
    }
    CompletionRequest {
        model: String::new(), // backends resolve their configured model
        messages: vec![
            Message {
                role: Role::System,
                content: system_prompt_for(level).to_string(),
            },
            Message {
                role: Role::User,
                content: body,
            },
        ],
        temperature: Some(0.2), // low variance: summaries feed summaries
        max_tokens: Some(max_tokens),
        stop: None,
        seed: None,
        tools: None,
    }
}

/// Parse and validate a job payload: it must deserialize into the shared
/// [`RlmJobPayload`] shape and carry at least one input (hashes or texts).
fn parse_inputs(raw: &str, job_id: i64) -> Result<RlmJobPayload> {
    let payload: RlmJobPayload =
        serde_json::from_str(raw).with_context(|| format!("invalid job payload for {job_id}"))?;
    if payload.hashes.is_empty() && payload.texts.is_empty() {
        anyhow::bail!("job {job_id} has no inputs");
    }
    Ok(payload)
}

/// Minimum accepted summary length: shorter output is treated as a model
/// failure and refused (protects the review queue from junk).
const MIN_SUMMARY_CHARS: usize = 20;

#[must_use]
fn summary_acceptable(summary: &str) -> bool {
    let s = summary.trim();
    !s.is_empty() && s.len() >= MIN_SUMMARY_CHARS
}

/// Process one claimed job end-to-end. Returns Ok(false) when the submission
/// was rejected as stale (cancelled meanwhile); Ok(true) on acceptance.
fn process(
    rt: &tokio::runtime::Runtime,
    client: &mut auth_client::AragsClient,
    backend: &std::sync::Arc<dyn LlmBackend>,
    model_name: &str,
    vol: &VolunteerConfig,
    session_token: &Arc<Mutex<String>>,
    job: ClaimedJob,
    _once: bool,
) -> Result<bool> {
    let payload = parse_inputs(&job.payload, job.id)?;

    let mut request = build_request(job.level, &job.subject, &payload, vol.max_tokens_per_job);
    request.model = model_name.to_string();
    tracing::info!(
        job_id = job.id,
        project = %job.project,
        level = job.level,
        subject = %job.subject,
        inputs = payload.texts.len(),
        "synthesizing rlm summary"
    );

    let response = rt
        .block_on(async { backend.complete(request).await })
        .inspect_err(|e| {
            tracing::warn!(error = %e, job_id = job.id, "LLM synthesis failed");
        })?;

    let summary = response.content.trim().to_string();
    if !summary_acceptable(&summary) {
        anyhow::bail!("model produced an implausibly short summary; refusing to submit");
    }

    // Attest the submission (issue `agnostic-rlm-rs-64af`): HMAC-SHA256 over the
    // session token so the server can verify non-admin volunteers before counting
    // the candidate toward the BFT quorum. A missing session yields an empty HMAC
    // which the server rejects for quorum jobs.
    let session = session_token.lock().clone();
    let submission_hmac = arags_core::rlm_attestation::sign_rlm_submission(
        &session,
        job.id,
        job.generation,
        &summary,
    );

    let resp = rt
        .block_on(client.complete_rlm_job(CompleteRlmJobRequest {
            job_id: job.id,
            generation: job.generation,
            summary_text: summary.clone(),
            model: response.model,
            template_version: if payload.template_version.is_empty() {
                "unknown".into()
            } else {
                payload.template_version.clone()
            },
            token_count: i64::from(response.usage.total_tokens),
            submission_hmac,
        }))?
        .into_inner();

    if resp.accepted {
        tracing::info!(
            job_id = job.id,
            node_id = %resp.node_id,
            auto_approved = resp.auto_approved,
            "rlm summary accepted"
        );
        Ok(true)
    } else {
        tracing::warn!(job_id = job.id, reason = %resp.reason, "rlm submission rejected");
        Ok(false)
    }
}

#[cfg(test)]
mod tests;
