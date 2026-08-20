#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use arlm_search::bm25::Bm25Search;
use arlm_search::decay::DecayConfig;
use arlm_search::hybrid::HybridSearch;
use arlm_search::types::{HybridResult, SearchOptions, SearchTier};
use arlm_storage::Storage;
use arlm_storage::sqlite::buffers::NewBuffer;
use std::collections::HashMap;
use tempfile::TempDir;

fn setup() -> (HybridSearch, Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let bm25 = Bm25Search::new(&storage).unwrap();
    let hybrid = HybridSearch::new(bm25, None, None);
    (hybrid, storage, tmp)
}

#[test]
fn test_rrf_fuse_single_list() {
    let results = vec![
        HybridResult {
            chunk_id: 1,
            score: 0.9,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.8,
            is_summary: false,
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
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.8,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 3,
            score: 0.7,
            is_summary: false,
        },
    ];

    let list2 = vec![
        HybridResult {
            chunk_id: 2,
            score: 0.95,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 1,
            score: 0.85,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 4,
            score: 0.75,
            is_summary: false,
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
            is_summary: false,
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
        is_summary: false,
    }];
    let list2 = vec![HybridResult {
        chunk_id: 2,
        score: 0.9,
        is_summary: false,
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
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.8,
            is_summary: false,
        },
    ];

    let list2 = vec![
        HybridResult {
            chunk_id: 1,
            score: 0.95,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 3,
            score: 0.7,
            is_summary: false,
        },
    ];

    let fused = HybridSearch::rrf_fuse(&[list1, list2], 10, 60.0);

    assert_eq!(fused[0].chunk_id, 1);
}

#[test]
fn test_rrf_fuse_bm25_entity_fusion() {
    let bm25 = vec![
        HybridResult {
            chunk_id: 1,
            score: 0.9,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.8,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 3,
            score: 0.7,
            is_summary: false,
        },
    ];

    let entity = vec![
        HybridResult {
            chunk_id: 2,
            score: 0.95,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 4,
            score: 0.85,
            is_summary: false,
        },
    ];

    let fused = HybridSearch::rrf_fuse(&[bm25, entity], 10, 60.0);

    assert_eq!(fused[0].chunk_id, 2);
    assert_eq!(fused.len(), 4);
}

#[test]
fn test_apply_decay_no_ages() {
    let (hybrid, _storage, _tmp) = setup();

    let results = vec![
        HybridResult {
            chunk_id: 1,
            score: 1.0,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.5,
            is_summary: false,
        },
    ];

    let decayed = hybrid.apply_decay(results, &HashMap::new());
    assert!((decayed[0].score - 1.0).abs() < f32::EPSILON);
    assert!((decayed[1].score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_apply_decay_with_ages() {
    let (hybrid, _storage, _tmp) = setup();
    let hybrid = hybrid.with_decay(DecayConfig::new(0.01));

    let results = vec![
        HybridResult {
            chunk_id: 1,
            score: 1.0,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 1.0,
            is_summary: false,
        },
    ];

    let mut ages = HashMap::new();
    ages.insert(1, 0.0); // fresh
    ages.insert(2, 69.0); // ~50% decay

    let decayed = hybrid.apply_decay(results, &ages);
    assert_eq!(decayed[0].chunk_id, 1);
    assert!((decayed[0].score - 1.0).abs() < 0.01);
    assert_eq!(decayed[1].chunk_id, 2);
    assert!((decayed[1].score - 0.5).abs() < 0.05);
}

#[test]
fn test_apply_decay_reorders_by_freshness() {
    let (hybrid, _storage, _tmp) = setup();
    let hybrid = hybrid.with_decay(DecayConfig::new(0.01));

    let results = vec![
        HybridResult {
            chunk_id: 2,
            score: 0.9,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 1,
            score: 0.5,
            is_summary: false,
        },
    ];

    let mut ages = HashMap::new();
    ages.insert(1, 0.0); // fresh
    ages.insert(2, 200.0); // very old: exp(-0.01*200) = exp(-2) ≈ 0.135

    let decayed = hybrid.apply_decay(results, &ages);
    assert_eq!(decayed[0].chunk_id, 1);
    assert_eq!(decayed[1].chunk_id, 2);
}

#[test]
fn test_apply_decay_disabled() {
    let (hybrid, _storage, _tmp) = setup();
    let hybrid = hybrid.with_decay(DecayConfig::disabled());

    let results = vec![
        HybridResult {
            chunk_id: 1,
            score: 1.0,
            is_summary: false,
        },
        HybridResult {
            chunk_id: 2,
            score: 0.5,
            is_summary: false,
        },
    ];

    let mut ages = HashMap::new();
    ages.insert(1, 0.0);
    ages.insert(2, 1000.0);

    let decayed = hybrid.apply_decay(results, &ages);
    assert_eq!(decayed[0].chunk_id, 1);
    assert!((decayed[0].score - 1.0).abs() < f32::EPSILON);
    assert_eq!(decayed[1].chunk_id, 2);
    assert!((decayed[1].score - 0.5).abs() < f32::EPSILON);
}

#[test]
fn test_hybrid_search_new_default_decay() {
    let (hybrid, _storage, _tmp) = setup();
    assert!(hybrid.decay().enabled);
    assert!((hybrid.decay().lambda - 0.01).abs() < f64::EPSILON);
}

#[test]
fn test_search_all_empty_db() {
    let (hybrid, storage, _tmp) = setup();
    let results = hybrid.search_all("test", 10, &storage).unwrap();
    assert!(results.is_empty());
}

#[test]
fn test_hybrid_search_with_decay_builder() {
    let (hybrid, _storage, _tmp) = setup();
    let hybrid = hybrid.with_decay(DecayConfig::new(0.05));
    assert!((hybrid.decay().lambda - 0.05).abs() < f64::EPSILON);
}

#[tokio::test]
async fn test_dual_layer_summaries() {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    let buffer = storage
        .insert_buffer(&NewBuffer {
            name: "p".into(),
            path: "/p".into(),
        })
        .unwrap();
    let sid = storage
        .insert_summary(
            buffer,
            "zzmodule router guards the api endpoint",
            "module",
            None,
            None,
            0.9,
            None,
            None,
        )
        .unwrap();
    assert!(sid > 0);

    let direct = storage.search_summaries("zzmodule", buffer, 10).unwrap();
    assert!(
        !direct.is_empty(),
        "search_summaries should find the summary"
    );

    let bm25 = Bm25Search::new(&storage).unwrap();
    let hybrid = HybridSearch::new(bm25, None, None);
    let options = SearchOptions {
        tier: SearchTier::Entity,
        top_k: 10,
    };

    let results = hybrid
        .search("zzmodule", None, buffer, &options, None, Some(&storage))
        .await
        .unwrap();

    let summaries: Vec<&HybridResult> = results.iter().filter(|r| r.is_summary).collect();
    assert!(!summaries.is_empty(), "expected a summary result");
    assert!(summaries.iter().all(|r| r.chunk_id > 0));
}
