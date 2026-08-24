#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use arags_core::logging::*;

/// `init_logging` is guarded by `Once`, so verbose state depends on which
/// test initializes first. Run all assertions in a single deterministic test.
#[test]
fn test_logging_init_timer_and_verbose() {
    init_logging(false);
    let timer = ScopedTimer::new("test_operation");
    std::thread::sleep(std::time::Duration::from_millis(10));
    assert!(timer.elapsed_ms() >= 10);
    assert!(!is_verbose());

    // Second call must be a no-op (Once): verbose state stays false.
    init_logging(true);
    assert!(!is_verbose());
    assert!(ScopedTimer::new_verbose("verbose_only").is_none());
}
