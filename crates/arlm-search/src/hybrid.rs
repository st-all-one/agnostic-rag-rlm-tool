use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use arlm_llm::{CompletionRequest, LlmBackend, Message, Role};
use arlm_storage::Storage;

use crate::bm25::Bm25Search;
use crate::context::build_search_results;
use crate::decay::DecayConfig;
use crate::entity::EntitySearch;
use crate::semantic::SemanticSearch;
use crate::types::{HybridResult, SearchOptions, SearchResult, SearchTier};

const RERANK_SYSTEM_PROMPT: &str = "You are a search relevance reranker. Rank the given candidate chunks by how relevant they are to the query. Respond with ONLY the chunk IDs in order of relevance, one ID per line, most relevant first. Do not include any other text.";

const RERANK_MODEL: &str = "rerank";

const RERANK_SNIPPET_LEN: usize = 200;

const RERANK_MAX_TOKENS: u32 = 256;

pub struct HybridSearch {
    bm25: Bm25Search,
    entity: Option<EntitySearch>,
    semantic: Option<SemanticSearch>,
    llm_backend: Option<Arc<dyn LlmBackend + Send + Sync>>,
    rrf_k: f32,
    decay: DecayConfig,
}

impl HybridSearch {
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

    /// Apply salience decay to results using chunk ages (in hours).
    ///
    /// Mutates scores in-place: `score *= exp(-lambda * age_hours)`.
    /// Re-sorts by decayed score and returns the results.
    #[must_use]
    pub fn apply_decay(
        &self,
        mut results: Vec<HybridResult>,
        chunk_ages: &HashMap<i64, f32>,
    ) -> Vec<HybridResult> {
        if !self.decay.enabled || chunk_ages.is_empty() {
            return results;
        }

        for r in &mut results {
            if let Some(&age) = chunk_ages.get(&r.chunk_id) {
                r.score = self.decay.score(r.score, age);
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    /// Search using BM25 only (tier 0), with optional salience decay.
    ///
    /// Pass `chunk_ages` (`chunk_id` -> age in hours) to apply decay.
    /// Pass `None` to skip decay.
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25 query fails.
    pub fn search_fts(
        &self,
        query: &str,
        buffer_id: i64,
        top_k: usize,
        chunk_ages: Option<&HashMap<i64, f32>>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();

        let bm25_results = self.bm25.search(query, buffer_id, top_k)?;

        let mut results: Vec<HybridResult> = bm25_results
            .into_iter()
            .map(|r| HybridResult {
                chunk_id: r.chunk_id,
                #[allow(clippy::cast_possible_truncation)]
                score: r.score as f32,
            })
            .collect();

        if let Some(ages) = chunk_ages {
            results = self.apply_decay(results, ages);
        }

        tracing::info!(
            tier = "fts",
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }

    /// Hybrid search with tier support, RRF fusion, and optional salience decay.
    ///
    /// - Tier 0 (Fts): BM25 only
    /// - Tier 1 (Entity, default): BM25 + entity RRF
    /// - Tier 2 (Vector): BM25 + entity + vector RRF
    /// - Tier 3 (LlmRerank): Tier 2 + LLM rerank (requires a configured
    ///   `llm_backend` via [`Self::with_llm_backend`] and `storage` to hydrate
    ///   candidate snippets)
    ///
    /// Pass `chunk_ages` (`chunk_id` -> age in hours) to apply decay after fusion.
    /// Pass `None` to skip decay.
    /// Pass `storage` (`Some`) to enable Tier 3 LLM reranking; when `None`, the
    /// fused results are returned as-is even if a backend is configured.
    ///
    /// # Errors
    ///
    /// Returns an error if the BM25, entity, or semantic query fails.
    pub async fn search(
        &self,
        query: &str,
        query_vector: Option<&[f32]>,
        buffer_id: i64,
        options: &SearchOptions,
        chunk_ages: Option<&HashMap<i64, f32>>,
        storage: Option<&Storage>,
    ) -> Result<Vec<HybridResult>> {
        let start = Instant::now();
        let k = options.top_k;

        let mut scores: HashMap<i64, f32> = HashMap::new();

        // Tier 0: BM25 always runs
        let bm25_results = self.bm25.search(query, buffer_id, k * 3)?;
        for (rank, result) in bm25_results.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score;
        }

        // Tier 1+: entity search
        if matches!(
            options.tier,
            SearchTier::Entity | SearchTier::Vector | SearchTier::LlmRerank
        ) {
            if let Some(entity_search) = &self.entity {
                let entities = EntitySearch::extract_query_entities(query);
                if !entities.is_empty() {
                    match entity_search.search(&entities, buffer_id, k * 3) {
                        Ok(entity_results) => {
                            for (rank, result) in entity_results.iter().enumerate() {
                                #[allow(clippy::cast_precision_loss)]
                                let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
                                *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "entity search failed, falling back to BM25 only"
                            );
                        }
                    }
                }
            }
        }

        // Tier 2+: vector search
        if matches!(options.tier, SearchTier::Vector | SearchTier::LlmRerank) {
            if let Some(sem) = &self.semantic {
                if let Some(vec) = query_vector {
                    match sem.search(vec, buffer_id, k * 3).await {
                        Ok(sem_results) => {
                            for (rank, result) in sem_results.iter().enumerate() {
                                #[allow(clippy::cast_precision_loss)]
                                let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
                                let cid =
                                    i64::try_from(result.chunk_id).context("chunk_id overflow")?;
                                *scores.entry(cid).or_insert(0.0) += rrf_score;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "semantic search failed, falling back to BM25 only"
                            );
                        }
                    }
                }
            }
        }

        let mut results: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results.truncate(k);

        if let Some(ages) = chunk_ages {
            results = self.apply_decay(results, ages);
            results.truncate(k);
        }

        // Tier 3: LLM rerank (requires a configured backend + storage)
        if matches!(options.tier, SearchTier::LlmRerank) {
            if let Some(storage) = storage {
                match self.rerank_with_llm(storage, query, results.clone()).await {
                    Ok(reranked) => results = reranked,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "LLM rerank failed, falling back to fused results"
                        );
                    }
                }
            } else {
                tracing::warn!("LLM rerank requires storage access, falling back to fused results");
            }
        }

        tracing::info!(
            tier = %options.tier,
            results_count = results.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "hybrid search completed"
        );

        Ok(results)
    }

    /// Reorder fused results by asking the LLM backend to rank candidate chunks.
    ///
    /// Builds a prompt containing the query and each candidate's chunk id plus a
    /// text snippet, calls the backend, parses the returned id ordering, and
    /// reorders the candidates accordingly. Candidate ids not mentioned in the
    /// response are appended at the end in their original order.
    ///
    /// # Errors
    ///
    /// Returns an error if the LLM completion fails.
    pub async fn llm_rerank(
        candidates: Vec<SearchResult>,
        query: &str,
        backend: &dyn LlmBackend,
    ) -> Result<Vec<SearchResult>> {
        if candidates.is_empty() {
            return Ok(candidates);
        }

        let prompt = build_rerank_prompt(query, &candidates);

        let request = CompletionRequest {
            model: RERANK_MODEL.to_string(),
            messages: vec![
                Message {
                    role: Role::System,
                    content: RERANK_SYSTEM_PROMPT.to_string(),
                },
                Message {
                    role: Role::User,
                    content: prompt,
                },
            ],
            temperature: Some(0.0),
            max_tokens: Some(RERANK_MAX_TOKENS),
            stop: None,
        };

        let response = backend
            .complete(request)
            .await
            .context("LLM rerank completion failed")?;

        let ranked_ids = parse_rerank_order(&response.content, &candidates);
        let reranked = reorder_candidates(candidates, &ranked_ids);

        tracing::info!(
            backend = %backend.name(),
            candidates = reranked.len(),
            "llm rerank completed"
        );

        Ok(reranked)
    }

    async fn rerank_with_llm(
        &self,
        storage: &Storage,
        query: &str,
        results: Vec<HybridResult>,
    ) -> Result<Vec<HybridResult>> {
        let Some(backend) = &self.llm_backend else {
            return Ok(results);
        };

        let search_results = build_search_results(storage, &results)?;

        let reranked = Self::llm_rerank(search_results, query, backend.as_ref()).await?;

        Ok(reranked
            .into_iter()
            .map(|s| HybridResult {
                chunk_id: s.chunk_id,
                score: s.score,
            })
            .collect())
    }

    /// Reciprocal Rank Fusion of multiple result lists.
    #[must_use]
    pub fn rrf_fuse(results_list: &[Vec<HybridResult>], top_k: usize, k: f32) -> Vec<HybridResult> {
        let mut scores: HashMap<i64, f32> = HashMap::new();

        for results in results_list {
            for (rank, result) in results.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score;
            }
        }

        let mut fused: Vec<HybridResult> = scores
            .into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        fused.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        fused.truncate(top_k);
        fused
    }
}

fn build_rerank_prompt(query: &str, candidates: &[SearchResult]) -> String {
    let mut prompt = format!("Query: {query}\n\nCandidates:\n");
    for c in candidates {
        let snippet: String = c.content.chars().take(RERANK_SNIPPET_LEN).collect();
        let _ = writeln!(prompt, "ID {} [{}]: {snippet}", c.chunk_id, c.file_path);
    }
    prompt.push_str("\nRank the IDs by relevance (most relevant first), one ID per line.");
    prompt
}

fn parse_rerank_order(response: &str, candidates: &[SearchResult]) -> Vec<i64> {
    let known: HashSet<i64> = candidates.iter().map(|c| c.chunk_id).collect();
    let mut ordered = Vec::new();
    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Try patterns: "1. 42", "ID 42", "42", "rank 1: id=42", "42 (score)"
        for token in
            line.split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ')' || ch == '(')
        {
            let token = token.trim_matches(|ch: char| !ch.is_ascii_digit());
            if token.is_empty() {
                continue;
            }
            if let Ok(id) = token.parse::<i64>() {
                if known.contains(&id) && !ordered.contains(&id) {
                    ordered.push(id);
                }
            }
        }
    }
    ordered
}

fn reorder_candidates(candidates: Vec<SearchResult>, ranked_ids: &[i64]) -> Vec<SearchResult> {
    let mut by_id: HashMap<i64, SearchResult> =
        candidates.into_iter().map(|c| (c.chunk_id, c)).collect();
    let mut reordered: Vec<SearchResult> = ranked_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect();
    // Append remaining candidates (not in ranked_ids) at the end in their original order
    let mut remaining: Vec<SearchResult> = by_id.into_values().collect();
    remaining.sort_by_key(|c| c.chunk_id);
    reordered.append(&mut remaining);
    reordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rrf_fuse_single_list() {
        let results = vec![
            HybridResult {
                chunk_id: 1,
                score: 0.9,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.8,
            },
        ];

        let fused = HybridSearch::rrf_fuse(&[results], 10, 60.0);
        assert_eq!(fused.len(), 2);
        assert_eq!(fused[0].chunk_id, 1);
    }

    #[test]
    fn test_rrf_fuse_multiple_lists() {
        let list1 = vec![
            HybridResult {
                chunk_id: 1,
                score: 0.9,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.8,
            },
            HybridResult {
                chunk_id: 3,
                score: 0.7,
            },
        ];

        let list2 = vec![
            HybridResult {
                chunk_id: 2,
                score: 0.95,
            },
            HybridResult {
                chunk_id: 1,
                score: 0.85,
            },
            HybridResult {
                chunk_id: 4,
                score: 0.75,
            },
        ];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);

        assert_eq!(fused.len(), 4);

        let chunk_ids: Vec<i64> = fused.iter().map(|r| r.chunk_id).collect();
        assert!(chunk_ids.contains(&1));
        assert!(chunk_ids.contains(&2));
        assert!(chunk_ids.contains(&3));
        assert!(chunk_ids.contains(&4));

        let scores: Vec<f32> = fused.iter().map(|r| r.score).collect();
        assert!(scores[0] >= scores[1]);
        assert!(scores[1] >= scores[2]);
    }

    #[test]
    fn test_rrf_fuse_top_k_limit() {
        let results: Vec<HybridResult> = (0..20)
            .map(|i| HybridResult {
                chunk_id: i,
                score: 1.0 - i as f32 * 0.01,
            })
            .collect();

        let fused = HybridSearch::rrf_fuse(&[results], 5, 60.0);
        assert_eq!(fused.len(), 5);
    }

    #[test]
    fn test_rrf_fuse_empty() {
        let fused = HybridSearch::rrf_fuse(&[], 10, 60.0);
        assert!(fused.is_empty());
    }

    #[test]
    fn test_rrf_fuse_disjoint_results() {
        let list1 = vec![HybridResult {
            chunk_id: 1,
            score: 0.9,
        }];
        let list2 = vec![HybridResult {
            chunk_id: 2,
            score: 0.9,
        }];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);
        assert_eq!(fused.len(), 2);

        let s1 = fused.iter().find(|r| r.chunk_id == 1).unwrap().score;
        let s2 = fused.iter().find(|r| r.chunk_id == 2).unwrap().score;
        assert!((s1 - s2).abs() < f32::EPSILON);
    }

    #[test]
    fn test_rrf_fuse_overlapping_high_rank_wins() {
        let list1 = vec![
            HybridResult {
                chunk_id: 1,
                score: 0.9,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.8,
            },
        ];

        let list2 = vec![
            HybridResult {
                chunk_id: 1,
                score: 0.95,
            },
            HybridResult {
                chunk_id: 3,
                score: 0.7,
            },
        ];

        let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);

        assert_eq!(fused[0].chunk_id, 1);
    }

    #[test]
    fn test_rrf_fuse_bm25_entity_fusion() {
        // Simulate BM25 results (chunks 1,2,3)
        let bm25 = vec![
            HybridResult {
                chunk_id: 1,
                score: 0.9,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.8,
            },
            HybridResult {
                chunk_id: 3,
                score: 0.7,
            },
        ];

        // Entity search finds chunk 2 and 4 (overlap on 2)
        let entity = vec![
            HybridResult {
                chunk_id: 2,
                score: 0.95,
            },
            HybridResult {
                chunk_id: 4,
                score: 0.85,
            },
        ];

        let fused = HybridSearch::rrf_fuse(&[bm25, entity], 10, 60.0);

        // chunk 2 should be #1 (appears in both lists at high rank)
        assert_eq!(fused[0].chunk_id, 2);
        // All 4 unique chunks present
        assert_eq!(fused.len(), 4);
    }

    // --- Decay-specific tests ---

    #[test]
    fn test_apply_decay_no_ages() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None);

        let results = vec![
            HybridResult {
                chunk_id: 1,
                score: 1.0,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.5,
            },
        ];

        let decayed = hybrid.apply_decay(results, &HashMap::new());
        assert!((decayed[0].score - 1.0).abs() < f32::EPSILON);
        assert!((decayed[1].score - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_apply_decay_with_ages() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None).with_decay(DecayConfig::new(0.01));

        let results = vec![
            HybridResult {
                chunk_id: 1,
                score: 1.0,
            },
            HybridResult {
                chunk_id: 2,
                score: 1.0,
            },
        ];

        let mut ages = HashMap::new();
        ages.insert(1, 0.0); // fresh
        ages.insert(2, 69.0); // ~50% decay

        let decayed = hybrid.apply_decay(results, &ages);
        // chunk 1 should be ranked first (fresh), chunk 2 should have ~0.5 score
        assert_eq!(decayed[0].chunk_id, 1);
        assert!((decayed[0].score - 1.0).abs() < 0.01);
        assert_eq!(decayed[1].chunk_id, 2);
        assert!((decayed[1].score - 0.5).abs() < 0.05);
    }

    #[test]
    fn test_apply_decay_reorders_by_freshness() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None).with_decay(DecayConfig::new(0.01));

        // chunk 2 has higher base score but is old; chunk 1 is fresh
        let results = vec![
            HybridResult {
                chunk_id: 2,
                score: 0.9,
            },
            HybridResult {
                chunk_id: 1,
                score: 0.5,
            },
        ];

        let mut ages = HashMap::new();
        ages.insert(1, 0.0); // fresh
        ages.insert(2, 200.0); // very old: exp(-0.01*200) = exp(-2) ≈ 0.135

        let decayed = hybrid.apply_decay(results, &ages);
        // chunk 1 should now be first despite lower base score
        assert_eq!(decayed[0].chunk_id, 1);
        assert_eq!(decayed[1].chunk_id, 2);
    }

    #[test]
    fn test_apply_decay_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None).with_decay(DecayConfig::disabled());

        let results = vec![
            HybridResult {
                chunk_id: 1,
                score: 1.0,
            },
            HybridResult {
                chunk_id: 2,
                score: 0.5,
            },
        ];

        let mut ages = HashMap::new();
        ages.insert(1, 0.0);
        ages.insert(2, 1000.0);

        let decayed = hybrid.apply_decay(results, &ages);
        // No decay applied, order preserved
        assert_eq!(decayed[0].chunk_id, 1);
        assert!((decayed[0].score - 1.0).abs() < f32::EPSILON);
        assert_eq!(decayed[1].chunk_id, 2);
        assert!((decayed[1].score - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_hybrid_search_new_default_decay() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None);
        assert!(hybrid.decay().enabled);
        assert!((hybrid.decay().lambda - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hybrid_search_with_decay_builder() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = arlm_storage::Storage::open(tmp.path()).unwrap();
        let bm25 = Bm25Search::new(&storage).unwrap();
        let hybrid = HybridSearch::new(bm25, None, None).with_decay(DecayConfig::new(0.05));
        assert!((hybrid.decay().lambda - 0.05).abs() < f64::EPSILON);
    }
}
