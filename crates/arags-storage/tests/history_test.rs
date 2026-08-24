#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::float_cmp
)]

use arags_storage::Storage;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

#[test]
fn test_insert_history() {
    let (storage, _tmp) = setup_storage();
    let id = storage
        .insert_history(
            None,
            "bug in login",
            Some("search"),
            Some(5),
            Some(100),
            Some("opencode"),
        )
        .unwrap();
    assert!(id > 0);
    let entries = storage.get_history(None, 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].query, "bug in login");
}
