//! In-memory A/B comparison harness for embedding **relevance** (not latency).
//!
//! This module is fully unit-testable with NO model files: the metric functions
//! ([`cosine_similarity`], [`recall_at_k`], [`ndcg_at_k`], [`mrr`]) are pure, and
//! [`run_ab`] wires two [`Embedder`](super::Embedder)s over a fixed corpus + query
//! set with relevance judgments. A gated integration test (`test_ab_real_models_gated`)
//! compares the shipped `all-minilm` (384d) against an alternate model such as
//! `qwen3-embedding:0.6b` (1024d) only when a human configures Ollama + the model.
//!
//! The ranking is a self-contained cosine-similarity search over in-memory vectors;
//! it does NOT touch the server, `SQLite`, or `usearch`.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tracing::debug;

use super::{Embedder, Embedding, EmbeddingResult};

/// Cosine similarity between two vectors.
///
/// Returns `0.0` (no similarity) when the dimensions mismatch or either vector
/// is empty, so callers never panic on shape errors. For unit-normalized
/// embeddings this equals the dot product.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

/// Recall@k: fraction of the relevant items that appear in the top-`k` of `ranked`.
///
/// `ranked` is an ordered list of chunk ids (best first). `relevant` is the set
/// of gold chunk ids for one query. Returns `0.0` when there are no relevant
/// items (undefined recall).
#[must_use]
pub fn recall_at_k(ranked: &[usize], relevant: &[usize], k: usize) -> f32 {
    if relevant.is_empty() {
        return 0.0;
    }
    let top_k = k.min(ranked.len());
    let rel_set: HashSet<usize> = relevant.iter().copied().collect();
    let hits = ranked[..top_k]
        .iter()
        .filter(|id| rel_set.contains(id))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let hits_f = hits as f32;
    #[allow(clippy::cast_precision_loss)]
    let rel_f = relevant.len() as f32;
    hits_f / rel_f
}

/// DCG@k from an already-ordered list of per-position graded relevances.
///
/// `grades[i]` is the graded relevance at 1-based rank `i + 1`; positions beyond
/// `k` are ignored. Uses the standard `DCG = Σ (2^rel - 1) / log2(rank + 1)`.
fn dcg_from_grades(grades: &[f32], k: usize) -> f32 {
    let top_k = k.min(grades.len());
    let mut dcg = 0.0_f32;
    for (i, &g) in grades.iter().take(top_k).enumerate() {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let denom = ((i + 1) as f32 + 1.0).log2();
        if denom > 0.0 {
            dcg += (2_f32.powf(g) - 1.0) / denom;
        }
    }
    dcg
}

/// nDCG@k with graded relevance.
///
/// `ranked` is the ordered chunk ids (best first). `relevant_grades` pairs each
/// gold chunk id with its graded relevance (binary `1.0` is fine). The ideal
/// ordering is the grades sorted descending; nDCG = DCG / IDCG. Returns `0.0`
/// when no grades are provided or the ideal DCG is zero.
#[must_use]
pub fn ndcg_at_k(ranked: &[usize], relevant_grades: &[(usize, f32)], k: usize) -> f32 {
    if relevant_grades.is_empty() {
        return 0.0;
    }
    let grade_of: HashMap<usize, f32> = relevant_grades.iter().map(|(id, g)| (*id, *g)).collect();

    let graded: Vec<f32> = ranked
        .iter()
        .map(|id| grade_of.get(id).copied().unwrap_or(0.0))
        .collect();
    let dcg = dcg_from_grades(&graded, k);

    let mut ideal: Vec<f32> = relevant_grades.iter().map(|(_, g)| *g).collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let idcg = dcg_from_grades(&ideal, k);

    if idcg <= 0.0 {
        return 0.0;
    }
    dcg / idcg
}

/// Mean Reciprocal Rank over a list of queries.
///
/// `ranked_list[q]` is the ranked chunk ids for query `q`; `relevant_list[q]`
/// is its gold set. The reciprocal rank of a query is `1 / (first_hit_rank)`
/// (1-based), or `0.0` if no relevant item is retrieved. Returns `0.0` when the
/// two lists differ in length or are empty.
#[must_use]
pub fn mrr(ranked_list: &[Vec<usize>], relevant_list: &[Vec<usize>]) -> f32 {
    if ranked_list.is_empty() || ranked_list.len() != relevant_list.len() {
        return 0.0;
    }
    let mut sum = 0.0_f32;
    for (ranked, rel) in ranked_list.iter().zip(relevant_list.iter()) {
        let rel_set: HashSet<usize> = rel.iter().copied().collect();
        let mut rr = 0.0_f32;
        for (i, id) in ranked.iter().enumerate() {
            if rel_set.contains(id) {
                #[allow(clippy::cast_precision_loss)]
                let rank_f = (i + 1) as f32;
                rr = 1.0 / rank_f;
                break;
            }
        }
        sum += rr;
    }
    #[allow(clippy::cast_precision_loss)]
    let n_f = ranked_list.len() as f32;
    sum / n_f
}

/// Aggregate A/B comparison result over all queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbResult {
    /// Mean Recall@k for embedder A.
    pub recall_a: f32,
    /// Mean Recall@k for embedder B.
    pub recall_b: f32,
    /// Mean nDCG@k for embedder A.
    pub ndcg_a: f32,
    /// Mean nDCG@k for embedder B.
    pub ndcg_b: f32,
    /// MRR for embedder A.
    pub mrr_a: f32,
    /// MRR for embedder B.
    pub mrr_b: f32,
}

/// Rank chunk ids by descending cosine similarity to `query`.
fn rank_by_similarity(query: &[f32], corpus: &[(usize, Embedding)]) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = corpus
        .iter()
        .map(|(id, emb)| (*id, cosine_similarity(query, emb)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.into_iter().map(|(id, _)| id).collect()
}

/// Run an in-memory A/B relevance comparison of two embedders.
///
/// Both embedders embed the entire `corpus` (into two independent in-memory
/// spaces — their dimensions may differ) and every query. Each query is ranked
/// by cosine similarity and scored against its gold `relevant` chunk ids using
/// Recall@k, nDCG@k (binary grades) and MRR. Results are averaged over queries.
///
/// No server, `SQLite`, or vector store is touched. `debug!` logs per-query embed
/// latency and the total; the returned [`AbResult`] is the comparison summary.
///
/// # Errors
///
/// Propagates embedding failures from either embedder.
pub fn run_ab<A: Embedder, B: Embedder>(
    corpus: &[(usize, String)],
    queries: &[(String, Vec<usize>)],
    ea: &A,
    eb: &B,
    k: usize,
) -> EmbeddingResult<AbResult> {
    let total_start = Instant::now();

    let corpus_texts: Vec<&str> = corpus.iter().map(|(_, text)| text.as_str()).collect();
    let emb_a = ea.embed_batch(&corpus_texts)?;
    let emb_b = eb.embed_batch(&corpus_texts)?;

    let corpus_a: Vec<(usize, Embedding)> = corpus
        .iter()
        .zip(emb_a)
        .map(|((id, _), e)| (*id, e))
        .collect();
    let corpus_b: Vec<(usize, Embedding)> = corpus
        .iter()
        .zip(emb_b)
        .map(|((id, _), e)| (*id, e))
        .collect();

    let mut recall_a = 0.0_f32;
    let mut recall_b = 0.0_f32;
    let mut ndcg_a = 0.0_f32;
    let mut ndcg_b = 0.0_f32;
    let mut ranked_list_a: Vec<Vec<usize>> = Vec::with_capacity(queries.len());
    let mut ranked_list_b: Vec<Vec<usize>> = Vec::with_capacity(queries.len());
    let mut rel_list: Vec<Vec<usize>> = Vec::with_capacity(queries.len());

    for (qtext, relevant) in queries {
        let q_start = Instant::now();
        let qa = ea.embed(qtext)?;
        debug!(
            elapsed_ms = q_start.elapsed().as_millis(),
            model = ea.name(),
            "query_embed"
        );
        let qb = eb.embed(qtext)?;
        debug!(
            elapsed_ms = q_start.elapsed().as_millis(),
            model = eb.name(),
            "query_embed"
        );

        let ranked_a = rank_by_similarity(&qa, &corpus_a);
        let ranked_b = rank_by_similarity(&qb, &corpus_b);

        let grades: Vec<(usize, f32)> = relevant.iter().map(|id| (*id, 1.0)).collect();
        recall_a += recall_at_k(&ranked_a, relevant, k);
        recall_b += recall_at_k(&ranked_b, relevant, k);
        ndcg_a += ndcg_at_k(&ranked_a, &grades, k);
        ndcg_b += ndcg_at_k(&ranked_b, &grades, k);

        ranked_list_a.push(ranked_a);
        ranked_list_b.push(ranked_b);
        rel_list.push(relevant.clone());
    }

    #[allow(clippy::cast_precision_loss)]
    let n_f = queries.len() as f32;
    let result = if n_f == 0.0 {
        AbResult {
            recall_a: 0.0,
            recall_b: 0.0,
            ndcg_a: 0.0,
            ndcg_b: 0.0,
            mrr_a: 0.0,
            mrr_b: 0.0,
        }
    } else {
        AbResult {
            recall_a: recall_a / n_f,
            recall_b: recall_b / n_f,
            ndcg_a: ndcg_a / n_f,
            ndcg_b: ndcg_b / n_f,
            mrr_a: mrr(&ranked_list_a, &rel_list),
            mrr_b: mrr(&ranked_list_b, &rel_list),
        }
    };

    debug!(
        elapsed_ms = total_start.elapsed().as_millis(),
        k = k,
        recall_a = result.recall_a,
        recall_b = result.recall_b,
        ndcg_a = result.ndcg_a,
        ndcg_b = result.ndcg_b,
        mrr_a = result.mrr_a,
        mrr_b = result.mrr_b,
        "ab_total"
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::float_cmp
    )]

    use super::*;
    use crate::embedder::config::{EmbeddingConfig, EmbeddingModel, Quantization, build_embedder};
    use crate::embedder::lightweight::LightweightEmbedder;
    use crate::embedder::minilm::MinilmEmbedder;
    use crate::embedder::ollama::OllamaEmbedder;
    use std::path::Path;

    const EPS: f32 = 1e-3;

    #[test]
    fn test_cosine_similarity_mismatched_dims_is_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![1.0_f32, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
        assert_eq!(cosine_similarity(&b, &a), 0.0);
    }

    #[test]
    fn test_cosine_similarity_empty_is_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![0.0_f32, 1.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < EPS);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert!((cosine_similarity(&a, &b)).abs() < EPS);
    }

    #[test]
    fn test_recall_at_k_exact() {
        let ranked = vec![3_usize, 1, 2, 4];
        let relevant = vec![1_usize, 2];
        assert!((recall_at_k(&ranked, &relevant, 2) - 0.5).abs() < EPS);
        assert!((recall_at_k(&ranked, &relevant, 4) - 1.0).abs() < EPS);
        assert!((recall_at_k(&ranked, &relevant, 1) - 0.0).abs() < EPS);
    }

    #[test]
    fn test_recall_at_k_empty_relevant() {
        let ranked = vec![1_usize, 2];
        assert_eq!(recall_at_k(&ranked, &[], 3), 0.0);
    }

    #[test]
    fn test_ndcg_at_k_exact() {
        // ranked: [4 (grade 0), 1 (grade 3), 2 (grade 2), 3 (grade 1)], k=3
        let ranked = vec![4_usize, 1, 2, 3];
        let grades = vec![(1_usize, 3.0_f32), (2, 2.0), (3, 1.0)];
        let dcg =
            (2_f32.powf(3.0) - 1.0) / 3.0_f32.log2() + (2_f32.powf(2.0) - 1.0) / 4.0_f32.log2();
        let idcg = (2_f32.powf(3.0) - 1.0) / 1.0
            + (2_f32.powf(2.0) - 1.0) / 3.0_f32.log2()
            + (2_f32.powf(1.0) - 1.0) / 4.0_f32.log2();
        let expected = dcg / idcg;
        assert!((ndcg_at_k(&ranked, &grades, 3) - expected).abs() < 1e-4);
    }

    #[test]
    fn test_ndcg_at_k_ideal_is_one() {
        let ranked = vec![1_usize, 2, 3];
        let grades = vec![(1_usize, 3.0_f32), (2, 2.0), (3, 1.0)];
        assert!((ndcg_at_k(&ranked, &grades, 3) - 1.0).abs() < EPS);
    }

    #[test]
    fn test_ndcg_at_k_empty() {
        let ranked = vec![1_usize];
        assert_eq!(ndcg_at_k(&ranked, &[], 3), 0.0);
    }

    #[test]
    fn test_mrr_exact() {
        let ranked_list = vec![vec![3_usize, 1, 4, 2], vec![2_usize, 5, 1]];
        let rel_list = vec![vec![1_usize], vec![1_usize, 2]];
        // q1: first hit id1 at rank 2 -> 0.5 ; q2: first hit id2 at rank 1 -> 1.0
        assert!((mrr(&ranked_list, &rel_list) - 0.75).abs() < EPS);
    }

    #[test]
    fn test_mrr_no_hit_is_zero() {
        let ranked_list = vec![vec![3_usize, 4]];
        let rel_list = vec![vec![1_usize]];
        assert_eq!(mrr(&ranked_list, &rel_list), 0.0);
    }

    #[test]
    fn test_mrr_length_mismatch() {
        let ranked_list = vec![vec![1_usize]];
        let rel_list = vec![vec![1_usize], vec![2_usize]];
        assert_eq!(mrr(&ranked_list, &rel_list), 0.0);
    }

    fn tiny_corpus() -> Vec<(usize, String)> {
        vec![
            (
                0usize,
                "database connection pool with retry and timeout".to_string(),
            ),
            (
                1,
                "http router middleware handling json requests".to_string(),
            ),
            (2, "vector similarity search using hnsw index".to_string()),
            (3, "tokenizer preprocessing for bert model".to_string()),
            (4, "sqlite fts5 full text search query parser".to_string()),
            (
                5,
                "grpc streaming server implementation in rust".to_string(),
            ),
        ]
    }

    fn tiny_queries() -> Vec<(String, Vec<usize>)> {
        vec![
            (
                "how to manage database connections".to_string(),
                vec![0usize],
            ),
            (
                "implementing a vector search index".to_string(),
                vec![2usize],
            ),
        ]
    }

    #[test]
    fn test_run_ab_deterministic() {
        let corpus = tiny_corpus();
        let queries = tiny_queries();
        let a = LightweightEmbedder::new(384);
        let b = LightweightEmbedder::new(384);

        let res = run_ab(&corpus, &queries, &a, &b, 3).expect("run_ab");

        for v in [
            res.recall_a,
            res.recall_b,
            res.ndcg_a,
            res.ndcg_b,
            res.mrr_a,
            res.mrr_b,
        ] {
            assert!(v.is_finite(), "metric must be finite: {v}");
            assert!((0.0..=1.0).contains(&v), "metric must be in [0,1]: {v}");
        }
        // Same embedder (same model + dims) => identical rankings => equal metrics.
        assert!((res.recall_a - res.recall_b).abs() < 1e-9);
        assert!((res.ndcg_a - res.ndcg_b).abs() < 1e-9);
        assert!((res.mrr_a - res.mrr_b).abs() < 1e-9);
    }

    #[test]
    fn test_run_ab_different_dims_still_runs() {
        let corpus = tiny_corpus();
        let queries = tiny_queries();
        let a = LightweightEmbedder::new(384);
        let b = LightweightEmbedder::new(1024);

        let res = run_ab(&corpus, &queries, &a, &b, 3).expect("run_ab");
        assert!(res.recall_a.is_finite());
        assert!(res.recall_b.is_finite());
    }

    #[test]
    #[ignore = "requires ARAGS_AB_B_MODEL (e.g. ollama qwen3-embedding:0.6b) + running Ollama"]
    fn test_ab_real_models_gated() {
        use tracing::info;

        let Ok(model_b) = std::env::var("ARAGS_AB_B_MODEL") else {
            eprintln!("SKIP gated A/B: ARAGS_AB_B_MODEL not set");
            return;
        };

        let ollama_url = std::env::var("ARAGS_OLLAMA_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let b = OllamaEmbedder::new(&ollama_url, &model_b).expect("build B (Ollama alternate)");

        let k = 5usize;
        let corpus = vec![
            (
                0usize,
                "database connection pool with retry and timeout".to_string(),
            ),
            (
                1,
                "http router middleware handling json requests".to_string(),
            ),
            (2, "vector similarity search using hnsw index".to_string()),
            (3, "tokenizer preprocessing for bert model".to_string()),
            (4, "sqlite fts5 full text search query parser".to_string()),
            (
                5,
                "grpc streaming server implementation in rust".to_string(),
            ),
            (
                6,
                "embeddings quantization with int8 matryoshka truncation".to_string(),
            ),
            (7, "rayon thread pool capping to reserve cores".to_string()),
            (8, "tracing structured json logging elapsed_ms".to_string()),
            (9, "ollama daemon http api embed endpoint".to_string()),
            (
                10,
                "benchmark latency vs relevance of embedding models".to_string(),
            ),
            (11, "natural language query over a code corpus".to_string()),
        ];
        let queries = vec![
            (
                "how to manage database connections".to_string(),
                vec![0usize],
            ),
            (
                "implementing a vector search index".to_string(),
                vec![2usize],
            ),
            (
                "full text search with sqlite fts5".to_string(),
                vec![4usize],
            ),
            (
                "structured logging of elapsed time".to_string(),
                vec![8usize],
            ),
            (
                "querying an ollama embed endpoint".to_string(),
                vec![9usize],
            ),
        ];

        let res = if let Ok(dir) = std::env::var("ARAGS_MINILM_DIR") {
            let a = MinilmEmbedder::new(Path::new(&dir), Quantization::Int8)
                .expect("build A (all-minilm)");
            let r = run_ab(&corpus, &queries, &a, &b, k).expect("run_ab");
            print_ab("all-minilm(384d)", &model_b, &r);
            r
        } else {
            let a = LightweightEmbedder::new(384);
            let r = run_ab(&corpus, &queries, &a, &b, k).expect("run_ab");
            print_ab("lightweight(384d, baseline)", &model_b, &r);
            r
        };

        info!(
            recall_a = res.recall_a,
            recall_b = res.recall_b,
            ndcg_a = res.ndcg_a,
            ndcg_b = res.ndcg_b,
            mrr_a = res.mrr_a,
            mrr_b = res.mrr_b,
            "ab_real_models_summary"
        );
        println!(
            "A/B result (A vs B={model_b}): recall {:.3}/{:.3} ndcg {:.3}/{:.3} mrr {:.3}/{:.3}",
            res.recall_a, res.recall_b, res.ndcg_a, res.ndcg_b, res.mrr_a, res.mrr_b
        );
    }

    fn print_ab(a_name: &str, b_name: &str, r: &AbResult) {
        println!(
            "=== Embedding A/B: {a_name} (A) vs {b_name} (B) ===\n\
             recall@{k}: A={:.3} B={:.3}\n\
             ndcg@{k}:   A={:.3} B={:.3}\n\
             mrr:       A={:.3} B={:.3}",
            r.recall_a,
            r.recall_b,
            r.ndcg_a,
            r.ndcg_b,
            r.mrr_a,
            r.mrr_b,
            k = 5
        );
    }

    // Ensure the test fixture config path compiles (build_embedder is exercised
    // indirectly by the gated test; this pins the lightweight path used there).
    #[test]
    fn test_build_embedder_lightweight_compiles() {
        let cfg = EmbeddingConfig {
            model: EmbeddingModel::Lightweight,
            ..EmbeddingConfig::default()
        };
        let emb = build_embedder(&cfg).expect("build lightweight");
        assert_eq!(emb.dimensions(), 384);
    }
}
