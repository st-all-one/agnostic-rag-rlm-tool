//! HMAC session-binding attestation for RLM volunteer submissions (issue
//! `agnostic-rlm-rs-64af`).
//!
//! Every non-admin volunteer submission is HMAC-SHA256 signed over a canonical
//! string `{job_id}.{generation}.{candidate_hash}` with a key derived from the
//! session token (`SHA256(session_token)`). The server re-derives the same tag
//! and verifies it (constant-time) before counting the submission toward the
//! BFT quorum, so a forged or tampered submission is rejected at the edge
//! rather than polluting the candidate pool.

use hmac::Mac;
use sha2::{Digest, Sha256};

/// Length (in hex chars) of the stable candidate hash component of the HMAC
/// message. Truncating the full SHA-256 hex keeps the canonical string compact
/// while remaining deterministic and collision-resistant for this purpose.
const CANDIDATE_HASH_HEX_LEN: usize = 16;

/// Compute a stable SHA-256 hex hash of `candidate_text`, truncated to
/// [`CANDIDATE_HASH_HEX_LEN`] hex characters, used as the final component of the
/// canonical HMAC message.
#[must_use]
fn candidate_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let full = hex::encode(digest);
    full[..CANDIDATE_HASH_HEX_LEN.min(full.len())].to_string()
}

/// Sign an RLM submission for a volunteer client.
///
/// The canonical message is `{job_id}.{generation}.{candidate_hash}` and the key
/// is `SHA256(session_token)`. Returns the hex HMAC-SHA256 tag that the server
/// verifies before staging the submission.
///
/// # Panics
///
/// Never: `Hmac::new_from_slice` only fails on oversized keys, which cannot
/// occur for a fixed 32-byte SHA-256 derived key; any error is mapped to an
/// empty tag (unreachable in practice).
#[must_use]
pub fn sign_rlm_submission(
    session_token: &str,
    job_id: i64,
    generation: i64,
    candidate_text: &str,
) -> String {
    let key = Sha256::digest(session_token.as_bytes());
    let candidate_hash = candidate_hash(candidate_text);
    let message = format!("{job_id}.{generation}.{candidate_hash}");
    let Ok(mut mac) = hmac::Hmac::<Sha256>::new_from_slice(&key) else {
        return String::new();
    };
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests;
