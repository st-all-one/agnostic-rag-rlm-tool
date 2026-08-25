//! Property-based tests for Reciprocal Rank Fusion (proptest, plan 021 §7.4).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::cast_precision_loss)]

use arags_search::hybrid::HybridSearch;
use arags_search::types::HybridResult;
use proptest::prelude::*;

fn ids() -> impl Strategy<Value = Vec<i64>> {
    proptest::collection::vec(any::<i64>(), 1..=24).prop_map(|v| {
        let mut seen = std::collections::HashSet::new();
        v.into_iter()
            .filter(|id| seen.insert(*id))
            .collect::<Vec<_>>()
    })
}

fn lists(max_lists: usize) -> impl Strategy<Value = Vec<Vec<i64>>> {
    proptest::collection::vec(ids(), 1..=max_lists)
}

fn fuse(list_ids: &[Vec<i64>], top_k: usize, k: f32) -> Vec<HybridResult> {
    let lists: Vec<Vec<HybridResult>> = list_ids
        .iter()
        .map(|ids| {
            ids.iter()
                .enumerate()
                .map(|(rank, id)| HybridResult {
                    chunk_id: *id,
                    score: 1.0 / (rank as f32 + 1.0),
                })
                .collect()
        })
        .collect();
    HybridSearch::rrf_fuse(&lists, top_k, k)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn fusion_is_deterministic_and_truncated(list_ids in lists(4), k in 1.0f32..=120.0) {
        let a = fuse(&list_ids, 8, k);
        let b = fuse(&list_ids, 8, k);
        prop_assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            prop_assert_eq!(x.chunk_id, y.chunk_id);
            prop_assert!((x.score - y.score).abs() <= f32::EPSILON);
        }
        prop_assert!(a.len() <= 8);
        // Descending score order.
        for w in a.windows(2) {
            prop_assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn fusion_keeps_every_item_when_top_k_is_large(list_ids in lists(3)) {
        let fused = fuse(&list_ids, 10_000, 60.0);
        let all: std::collections::HashSet<i64> =
            list_ids.iter().flatten().copied().collect();
        let got: std::collections::HashSet<i64> =
            fused.iter().map(|r| r.chunk_id).collect();
        prop_assert_eq!(all, got);
    }

    #[test]
    fn higher_rank_never_loses_to_lower_within_same_list(
        id_a in any::<i64>().prop_filter("distinct", |i| *i != 0 && *i != 1),
        tail in ids(),
    ) {
        // Same items; `id_a` ranked first in list A and last in list B.
        let mut with_tail = vec![id_a];
        with_tail.extend(tail.iter().copied());
        let first = vec![with_tail.clone()];
        let mut rotated = with_tail.clone();
        rotated.rotate_left(with_tail.len() - 1); // id_a goes last
        let second = vec![rotated];

        let fa = fuse(&first, 10_000, 60.0);
        let fb = fuse(&second, 10_000, 60.0);
        let sa = fa.iter().find(|r| r.chunk_id == id_a).unwrap().score;
        let sb = fb.iter().find(|r| r.chunk_id == id_a).unwrap().score;
        prop_assert!(
            sa > sb,
            "same item must score strictly higher when ranked first ({sa}) vs last ({sb})"
        );
    }

    #[test]
    fn scores_bounded_by_list_contributions(list_ids in lists(4), k in 8.0f32..=200.0) {
        let fused = fuse(&list_ids, 10_000, k);
        let max_possible = list_ids.len() as f32 / (k + 1.0);
        for r in &fused {
            prop_assert!(r.score > 0.0);
            prop_assert!(r.score <= max_possible + f32::EPSILON);
        }
    }
}
