use std::time::Duration;

use serde::Deserialize;
use ureq::Agent;

use super::{Embedder, Embedding, EmbeddingError, EmbeddingResult};

/// Embedder backed by an Ollama `/api/embed` endpoint.
///
/// Designed for lightweight local deployment: runs a small model such as
/// `nomic-embed-text-v2-moe` (a 768-dimensional mixture-of-experts model,
/// ~305M active parameters) on CPU with millisecond latency per chunk, instead
/// of a heavy transformer like `BgeM3`.
///
/// Nomic models expect a task prefix on the input (`search_query:` for queries,
/// `search_document:` for documents) — pass the appropriate prefix via
/// [`OllamaEmbedder::new`].
pub struct OllamaEmbedder {
    url: String,
    model: String,
    agent: Agent,
    dims: usize,
    prefix: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl OllamaEmbedder {
    /// # Panics
    ///
    /// Never panics; constructs an agent with a fixed timeout.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        model: impl Into<String>,
        dims: usize,
        prefix: impl Into<String>,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build();
        Self {
            url: url.into(),
            model: model.into(),
            agent,
            dims,
            prefix: prefix.into(),
        }
    }

    fn embed_one(&self, text: &str) -> EmbeddingResult<Embedding> {
        let payload = serde_json::json!({
            "model": self.model,
            "input": format!("{}{}", self.prefix, text),
        });
        let resp = self
            .agent
            .post(&format!("{}/api/embed", self.url))
            .send_json(payload)
            .map_err(|e| EmbeddingError::Ollama(format!("request failed: {e}")))?;
        let body: EmbedResponse = resp
            .into_json()
            .map_err(|e| EmbeddingError::Ollama(format!("invalid response: {e}")))?;
        let vector = body
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| EmbeddingError::Ollama("empty embeddings array".into()))?;
        if vector.len() != self.dims {
            return Err(EmbeddingError::DimensionMismatch {
                expected: self.dims,
                actual: vector.len(),
            });
        }
        Ok(vector)
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, text: &str) -> EmbeddingResult<Embedding> {
        self.embed_one(text)
    }

    fn embed_batch(&self, texts: &[&str]) -> EmbeddingResult<Vec<Embedding>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inputs: Vec<String> = texts
            .iter()
            .map(|t| format!("{}{}", self.prefix, t))
            .collect();
        let payload = serde_json::json!({
            "model": self.model,
            "input": inputs,
        });
        let resp = self
            .agent
            .post(&format!("{}/api/embed", self.url))
            .send_json(payload)
            .map_err(|e| EmbeddingError::Ollama(format!("request failed: {e}")))?;
        let body: EmbedResponse = resp
            .into_json()
            .map_err(|e| EmbeddingError::Ollama(format!("invalid response: {e}")))?;
        if body.embeddings.len() != texts.len() {
            return Err(EmbeddingError::Ollama(format!(
                "expected {} embeddings, got {}",
                texts.len(),
                body.embeddings.len()
            )));
        }
        for vector in &body.embeddings {
            if vector.len() != self.dims {
                return Err(EmbeddingError::DimensionMismatch {
                    expected: self.dims,
                    actual: vector.len(),
                });
            }
        }
        Ok(body.embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn name(&self) -> &'static str {
        "ollama"
    }
}
