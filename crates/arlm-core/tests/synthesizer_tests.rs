#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::*;

#[test]
fn test_build_children_block_completed() {
    let child = RlmNode::completed("c1", 1, "child task", "result text".to_string());
    let block = build_children_block(&[child]);
    assert!(block.contains("result text"));
    assert!(block.contains("completed"));
}

#[test]
fn test_build_children_block_failed() {
    let child = RlmNode::failed("c1", 1, "child task", "error msg".to_string());
    let block = build_children_block(&[child]);
    assert!(block.contains("FAILED"));
    assert!(block.contains("error msg"));
}

#[test]
fn test_build_children_block_cancelled() {
    let mut child = RlmNode::cancelled("c1", 1, "child task");
    child.partial_answer = Some("partial result".to_string());
    let block = build_children_block(&[child]);
    assert!(block.contains("CANCELLED"));
    assert!(block.contains("partial result"));
}

#[test]
fn test_build_children_block_skipped() {
    let child = RlmNode::skipped("c1", 1, "child task");
    let block = build_children_block(&[child]);
    assert!(block.contains("SKIPPED"));
}

#[test]
fn test_build_children_block_multiple() {
    let children = vec![
        RlmNode::completed("c1", 1, "t1", "r1".to_string()),
        RlmNode::failed("c2", 1, "t2", "e2".to_string()),
    ];
    let block = build_children_block(&children);
    assert!(block.contains("Child 1"));
    assert!(block.contains("Child 2"));
}
