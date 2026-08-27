#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;

#[test]
fn sign_is_deterministic_for_same_inputs() {
    let a = sign_rlm_submission("sess-token-xyz", 7, 3, "a summary draft");
    let b = sign_rlm_submission("sess-token-xyz", 7, 3, "a summary draft");
    assert_eq!(a, b, "same inputs must yield the same HMAC");
    assert_eq!(a.len(), 64, "hex HMAC-SHA256 is 64 chars");
}

#[test]
fn sign_changes_with_any_input_component() {
    let base = sign_rlm_submission("token", 1, 1, "text");
    // Different token.
    assert_ne!(
        base,
        sign_rlm_submission("token2", 1, 1, "text"),
        "different session token changes the tag"
    );
    // Different job id.
    assert_ne!(
        base,
        sign_rlm_submission("token", 2, 1, "text"),
        "different job_id changes the tag"
    );
    // Different generation.
    assert_ne!(
        base,
        sign_rlm_submission("token", 1, 2, "text"),
        "different generation changes the tag"
    );
    // Different candidate text.
    assert_ne!(
        base,
        sign_rlm_submission("token", 1, 1, "other text"),
        "different candidate text changes the tag"
    );
}

#[test]
fn sign_matches_manual_hmac_computation() {
    // Independent re-derivation to guard against silent regression.
    let token = "manual-token";
    let job_id = 42i64;
    let generation = 9i64;
    let text = "manual candidate summary";

    let key = Sha256::digest(token.as_bytes());
    let candidate_hash = {
        let full = hex::encode(Sha256::digest(text.as_bytes()));
        full[..16].to_string()
    };
    let message = format!("{job_id}.{generation}.{candidate_hash}");
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).unwrap();
    mac.update(message.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());

    assert_eq!(
        sign_rlm_submission(token, job_id, generation, text),
        expected
    );
}
