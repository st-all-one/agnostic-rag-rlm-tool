//! FTS5 query sanitisation helpers.
//!
//! User-supplied queries must never be bound verbatim to an FTS5 `MATCH`
//! clause: characters such as `-`, `:`, `"`, `*` and `^` are FTS5 operators
//! and can produce parse errors (e.g. `x-forwarded-for` is parsed as a column
//! filter, yielding `no such column: forwarded`). The helper below collapses
//! every non-alphanumeric/non-whitespace character to a space, producing a
//! safe AND-of-tokens query that FTS5 always accepts.

/// Sanitise a free-text query for use in an FTS5 `MATCH` parameter.
///
/// Only alphanumeric characters and whitespace are preserved; every other
/// character is replaced with a space. The result is safe to bind directly.
#[must_use]
pub fn sanitize_query(query: &str) -> String {
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
