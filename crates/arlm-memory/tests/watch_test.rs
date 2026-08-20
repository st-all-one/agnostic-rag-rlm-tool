#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::all,
    clippy::pedantic,
    clippy::nursery
)]

use arlm_memory::watch::*;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn test_watch_and_event() {
    let tmp = TempDir::new().unwrap();
    let opts = WatchOptions {
        debounce_ms: 10,
        recursive: true,
    };

    let handle = WatchMonitor::watch(tmp.path(), &opts).unwrap();

    // Create a file to trigger an event
    std::fs::write(tmp.path().join("new.txt"), "hello").unwrap();

    // Wait for the event with a timeout
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut event_received = false;

    while std::time::Instant::now() < deadline {
        if let Some(event) = handle.try_recv() {
            assert_eq!(event.kind, WatchEventKind::Created);
            event_received = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(event_received, "expected file creation event");
}

#[test]
fn test_watch_options_default() {
    let opts = WatchOptions::default();
    assert_eq!(opts.debounce_ms, 500);
    assert!(opts.recursive);
}
