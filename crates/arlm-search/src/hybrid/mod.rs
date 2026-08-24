pub mod fusion;
pub mod rrf;
pub mod search;

use crate::bm25::Bm25Search;
use crate::decay::DecayConfig;
use crate::entity::EntitySearch;
use crate::semantic::SemanticSearch;

/// Orchestrates hybrid search tiers (BM25 + entity + semantic) with Reciprocal
/// Rank Fusion (RRF) and optional salience decay.
pub struct HybridSearch {
    bm25: Bm25Search,
    entity: Option<EntitySearch>,
    semantic: Option<SemanticSearch>,
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
