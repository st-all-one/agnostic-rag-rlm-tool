use super::*;

#[test]
fn cosine_identical_is_one() {
    let v = vec![0.1, 0.2, -0.3, 0.4];
    assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
}

#[test]
fn cosine_orthogonal_is_zero() {
    assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-5);
}

#[test]
fn jaccard_half_overlap() {
    let a = vec!["c1".into(), "c2".into(), "c3".into()];
    let b = vec!["c1".into(), "c2".into(), "c4".into()];
    assert!((jaccard_similarity(&a, &b) - 0.5).abs() < 1e-6);
}

#[test]
fn jaccard_disjoint_is_zero() {
    let a = vec!["c1".into()];
    let b = vec!["c9".into()];
    assert_eq!(jaccard_similarity(&a, &b), 0.0);
}
