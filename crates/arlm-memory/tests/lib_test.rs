#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::{ScopedTimer, version};

#[test]
fn test_version() {
    assert!(!version().is_empty());
}

#[test]
fn test_scoped_timer() {
    let timer = ScopedTimer::new("test_op");
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert!(timer.elapsed_ms() >= 5);
}
