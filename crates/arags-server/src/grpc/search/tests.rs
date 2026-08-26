//! Unit tests for the unified query budget split (plan 023).

use super::split_summary_budget;

#[test]
fn ratio_split_with_plenty_of_summaries() {
    // 60% of 10 = 6 summaries, 4 chunk slots.
    assert_eq!(split_summary_budget(10, 0.6, 8), (6, 4));
}

#[test]
fn few_qualifying_summaries_leave_budget_for_chunks() {
    // Only 2 summaries qualify: they take 2 slots, chunks keep the rest.
    assert_eq!(split_summary_budget(10, 0.6, 2), (2, 8));
}

#[test]
fn no_qualifying_summaries_keeps_full_chunk_budget() {
    assert_eq!(split_summary_budget(10, 0.6, 0), (0, 10));
}

#[test]
fn zero_ratio_disables_fusion() {
    assert_eq!(split_summary_budget(10, 0.0, 8), (0, 10));
}

#[test]
fn at_least_one_chunk_slot_is_preserved() {
    // Degenerate ratio of 1.0 must not remove all real code from the answer.
    let (take, chunks) = split_summary_budget(10, 1.0, 20);
    assert!(chunks >= 1);
    assert_eq!(take + chunks, 10);
}

#[test]
fn tiny_budgets_do_not_split() {
    assert_eq!(split_summary_budget(1, 0.6, 5), (0, 1));
    assert_eq!(split_summary_budget(0, 0.6, 5), (0, 0));
}
