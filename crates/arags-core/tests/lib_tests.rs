#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use arags_core::*;

#[test]
fn test_version() {
    assert!(!version().is_empty());
}
