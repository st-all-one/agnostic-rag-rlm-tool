//! Similarity math for the semantic query-answer cache (plan 017).
//!
//! Cache lookup decides hit/miss/tier by question cosine similarity **plus** a
//! secondary check that defeats false positives (e.g. "login" vs "logout" have
//! nearby question vectors but disjoint provenance). These helpers are pure so
//! they can be unit-tested without storage or an embedder.

/// Cosine similarity of two vectors, clamped to `[-1, 1]`.
///
/// Returns `0.0` for zero vectors or mismatched lengths.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na * nb)).clamp(-1.0, 1.0)
}

/// Jaccard similarity of two string multisets, in `[0, 1]`:
/// `|a ∩ b| / |a ∪ b|`. Used as the secondary check on provenance chunk ids.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn jaccard_similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(String::as_str).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(String::as_str).collect();
    let inter = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    inter as f32 / union as f32
}

#[cfg(test)]
mod tests;
