#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arlm_core::logging::*;

#[test]
fn test_scoped_timer() {
    init_logging(false);
    let timer = ScopedTimer::new("test_operation");
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(timer.elapsed_ms() >= 10);
}

#[test]
fn test_is_verbose() {
    init_logging(true);
    assert!(is_verbose());
}
