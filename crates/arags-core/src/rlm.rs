//! Shared RLM domain constants and wire payloads.
//!
//! Single source of truth for values every side of the protocol must agree
//! on: the client ([`crate`]-based CLI volunteer), the server data plane
//! (`arags-server`) and persistence (`arags-storage`, which re-exports these).

use serde::{Deserialize, Serialize};

/// Default job lease in milliseconds (500s for every level).
pub const DEFAULT_RLM_LEASE_MS: i64 = 500_000;

/// Job priority ladder (lower value = processed first).
pub const PRIORITY_CANCELLED: i64 = 0;
/// Failed attempt returning to the queue soon.
pub const PRIORITY_RETRY: i64 = 1;
/// Parent-level rebuild triggered by a child completion.
pub const PRIORITY_CASCADE: i64 = 3;
/// Fresh L1 work from indexing.
pub const PRIORITY_FRESH: i64 = 5;
/// Exhausted `max_attempts`; parked at the back of the queue.
pub const PRIORITY_PARKED: i64 = 9;

/// JSON payload carried by an RLM job: input refs plus template metadata.
///
/// All fields default on deserialization so readers tolerate older payloads;
/// writers omit empty vectors to keep rows small.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RlmJobPayload {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunk_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    /// Template version the volunteer should apply.
    #[serde(default)]
    pub template_version: String,
    /// Hierarchy instructions: what this job summarizes (`file`, `theme`,
    /// `project`).
    #[serde(default)]
    pub subject_kind: String,
}

#[cfg(test)]
mod tests;
