pub mod fusion;
pub mod rerank;
pub mod rrf;
pub mod search;

use std::sync::Arc;

use arlm_llm::LlmBackend;

use crate::bm25::Bm25Search;
use crate::decay::DecayConfig;
use crate::entity::EntitySearch;
use crate::semantic::SemanticSearch;

const RERANK_SYSTEM_PROMPT: &str = "You are a search relevance reranker. Rank the given candidate chunks by how relevant they are to the query. Respond with ONLY the chunk IDs in order of relevance, one ID per line, most relevant first. Do not include any other text.";

const RERANK_MODEL: &str = "rerank";

const RERANK_SNIPPET_LEN: usize = 200;

const RERANK_MAX_TOKENS: u32 = 256;

/// Orchestrates multi-tier hybrid search with Reciprocal Rank Fusion (RRF),
/// optional salience decay, and optional LLM reranking.
pub struct HybridSearch {
    bm25: Bm25Search,
    entity: Option<EntitySearch>,
    semantic: Option<SemanticSearch>,
    llm_backend: Option<Arc<dyn LlmBackend + Send + Sync>>,
    rrf_k: f32,
    decay: DecayConfig,
}

impl HybridSearch {
    /// Create a new hybrid search instance.
    ///
    /// `entity` and `semantic` are optional tiers; when `None` those tiers are
    /// skipped during [`Self::search`].
    #[must_use]
    pub fn new(
        bm25: Bm25Search,
        entity: Option<EntitySearch>,
        semantic: Option<SemanticSearch>,
    ) -> Self {
        Self {
            bm25,
            entity,
            semantic,
            llm_backend: None,
            rrf_k: 60.0,
            decay: DecayConfig::default(),
        }
    }

    /// Builder: set the decay config for salience decay.
    #[must_use]
    pub fn with_decay(mut self, config: DecayConfig) -> Self {
        self.decay = config;
        self
    }

    /// Builder: set the LLM backend used for Tier 3 reranking.
    #[must_use]
    pub fn with_llm_backend(mut self, backend: Arc<dyn LlmBackend + Send + Sync>) -> Self {
        self.llm_backend = Some(backend);
        self
    }

    /// Set the decay config for salience decay.
    pub fn set_decay(&mut self, config: DecayConfig) {
        self.decay = config;
    }

    #[must_use]
    pub fn bm25(&self) -> &Bm25Search {
        &self.bm25
    }

    #[must_use]
    pub fn decay(&self) -> &DecayConfig {
        &self.decay
    }
}
