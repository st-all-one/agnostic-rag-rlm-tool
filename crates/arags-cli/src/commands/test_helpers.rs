//! Test-only helpers shared across `arags-cli` command tests.
//!
//! Provides a `MockLlmBackend` that returns canned completions without any
//! network or real LLM, enabling unit tests of the client-side digest/summary
//! paths (`digest_chunks`, `generate_summary`) and the CoT-stripping contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use arags_llm::{CompletionRequest, CompletionResponse, LlmBackend, LlmError, UsageSummary};
use async_trait::async_trait;

/// A canned completion returned by [`MockLlmBackend`].
#[derive(Clone)]
pub struct MockReply {
    /// Full completion text (may include leaked `<think>` to exercise stripping).
    pub content: String,
}

/// In-memory LLM backend used by client-path unit tests.
///
/// Wraps a fixed reply and records the last [`CompletionRequest`] it saw so
/// tests can assert on the prompt/model passed by the caller.
pub struct MockLlmBackend {
    reply: MockReply,
    last_request: Arc<Mutex<Option<CompletionRequest>>>,
}

impl MockLlmBackend {
    /// Build a mock that always returns `reply`.
    pub fn new(reply: MockReply) -> Self {
        Self {
            reply,
            last_request: Arc::new(Mutex::new(None)),
        }
    }

    /// Shared handle to the captured-last-request cell (for assertions).
    #[allow(dead_code)]
    pub fn last_request_handle(&self) -> Arc<Mutex<Option<CompletionRequest>>> {
        Arc::clone(&self.last_request)
    }
}

#[async_trait]
impl LlmBackend for MockLlmBackend {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        *self.last_request.lock().expect("mock lock poisoned") = Some(request);
        Ok(CompletionResponse {
            content: self.reply.content.clone(),
            model: "mock".to_string(),
            usage: UsageSummary::default(),
        })
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn default_model(&self) -> Option<String> {
        Some("mock".to_string())
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        Ok(())
    }
}

/// A clean reply containing the mandated `## Summary` section.
pub fn clean_summary_reply() -> MockReply {
    MockReply {
        content: "## Summary\n\nThe digest explains the wiring.\n\n## Key Findings / Artifacts\n\n- module A\n\n## Related\n\n- module B".to_string(),
    }
}

/// A reply that embeds leaked chain-of-thought before the real content.
pub fn cot_leak_reply() -> MockReply {
    MockReply {
        content: "<think>leaked reasoning that must not be stored</think>## Summary\n\nThe digest explains the wiring.\n\n## Key Findings / Artifacts\n\n- module A".to_string(),
    }
}

/// A clean digest (no CoT) for `digest_chunks`.
pub fn clean_digest_reply() -> MockReply {
    MockReply {
        content: "The answer is 42, per the source chunks.".to_string(),
    }
}

/// A digest reply that embeds leaked chain-of-thought.
pub fn cot_leak_digest_reply() -> MockReply {
    MockReply {
        content: "<think>internal monologue about the chunks</think>The answer is 42, per the source chunks.".to_string(),
    }
}
