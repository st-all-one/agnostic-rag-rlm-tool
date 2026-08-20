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

use arlm_storage::Storage;
use tempfile::TempDir;

fn setup_storage() -> (Storage, TempDir) {
    let tmp = TempDir::new().unwrap();
    let storage = Storage::open(tmp.path()).unwrap();
    (storage, tmp)
}

#[test]
fn test_insert_pattern() {
    let (storage, _tmp) = setup_storage();
    let id = storage
        .insert_pattern(
            None,
            Some("architectural"),
            "use of builder pattern",
            Some("Complex objects use builder pattern"),
            None,
            Some(0.85),
        )
        .unwrap();
    assert!(id > 0);
    let patterns = storage.get_patterns(None).unwrap();
    assert_eq!(patterns.len(), 1);
    assert_eq!(patterns[0].name, "use of builder pattern");
}
