//! Shared gRPC handler helpers.

use arags_proto::proto::SearchResult;

/// Sanitise a user query for FTS5 `MATCH`: keep only alphanumeric and
/// whitespace, collapsing everything else to a space.
#[must_use]
pub fn sanitize_fts(query: &str) -> String {
    query
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// Map hydrated `arags_search::SearchResult`s into the gRPC `SearchResult`
/// shape. Line numbers beyond `i32` clamp to `i32::MAX` (unrealistic for
/// real sources; avoids silent wraparound of a raw cast).
#[must_use]
pub fn to_proto_results(results: &[arags_search::SearchResult]) -> Vec<SearchResult> {
    let line = |v: i64| i32::try_from(v).unwrap_or(i32::MAX);
    results
        .iter()
        .map(|r| SearchResult {
            chunk_id: r.chunk_id,
            text: r.content.clone(),
            score: r.score,
            file_path: r.file_path.clone(),
            start_line: line(r.line_start),
            end_line: line(r.line_end),
            epoch: r.epoch,
            created_by: r.created_by.clone().unwrap_or_default(),
            model: r.model.clone().unwrap_or_default(),
            version: r.version,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn sanitize_keeps_words_collapses_symbols() {
        assert_eq!(sanitize_fts("foo AND bar"), "foo AND bar");
        assert_eq!(sanitize_fts("fn main() -> ! { }"), "fn main           ");
        assert_eq!(sanitize_fts("c++ & rust"), "c     rust");
        assert_eq!(sanitize_fts(""), "");
        // Unicode letters survive; symbols/punctuation collapse to spaces.
        assert_eq!(sanitize_fts("café ☕!"), "café   ");
    }
}
