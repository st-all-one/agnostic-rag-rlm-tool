#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;

#[test]
fn test_partition_files_round_robin_and_disjoint() {
    let files: Vec<PathBuf> = (0..7).map(|i| PathBuf::from(format!("f{i}"))).collect();
    let groups = partition_files(&files, 3);
    assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 7);
    assert_eq!(groups.len(), 3);
    let mut seen = std::collections::HashSet::new();
    for g in &groups {
        for f in g {
            assert!(seen.insert(f.clone()), "duplicate {f:?}");
        }
    }
    assert_eq!(partition_files(&[], 4).len(), 0);
    assert_eq!(partition_files(&files[..1], 8).len(), 1);
}

#[test]
fn test_zstd_roundtrip_shrinks_repetitive_text() {
    let text = "fn main() { println!(\"hello\"); }\n".repeat(100);
    let compressed =
        zstd::stream::encode_all(text.as_bytes(), UPLOAD_ZSTD_LEVEL).expect("compress");
    let decoded = zstd::stream::decode_all(&mut &compressed[..]).expect("decompress");
    assert_eq!(decoded, text.as_bytes());
    assert!(compressed.len() < text.len() / 10);
}
