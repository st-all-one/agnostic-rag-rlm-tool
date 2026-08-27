//! Post-processing of client-side LLM completions.
//!
//! Reasoning models (MiniCPM5, Qwen3-think, ...) leak chain-of-thought delimited
//! by `<think>...</think>` into the completion even when `think:false` was
//! requested. This module provides a defensive, panic-free stripper applied to
//! every client LLM result before it reaches the digest/summary stores.

/// Returns `true` if `bytes` starting at `i` begins a `<think>` (or `<THINK>`,
/// with optional inner whitespace) opening tag, and writes the byte offset just
/// past the closing `>` into `end`.
fn is_open_tag(bytes: &[u8], i: usize, end: &mut usize) -> bool {
    if i + 6 > bytes.len() || bytes[i] != b'<' {
        return false;
    }
    let seq = b"think";
    let mut k = i + 1;
    for &c in seq {
        let Some(&b) = bytes.get(k) else {
            return false;
        };
        if b != c && b != c.to_ascii_uppercase() {
            return false;
        }
        k += 1;
    }
    let mut m = k;
    while m < bytes.len()
        && (bytes[m] == b' ' || bytes[m] == b'\t' || bytes[m] == b'\n' || bytes[m] == b'\r')
    {
        m += 1;
    }
    if m < bytes.len() && bytes[m] == b'>' {
        *end = m + 1;
        true
    } else {
        false
    }
}

/// Returns `Some(offset)` just past the next `</think>` (case-insensitive,
/// optional inner whitespace) closing tag at or after `start`, else `None`.
fn find_close_tag(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 8 <= bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'/' {
            let seq = b"think";
            let mut k = i + 2;
            let mut ok = true;
            for &c in seq {
                let Some(&b) = bytes.get(k) else {
                    break;
                };
                if b != c && b != c.to_ascii_uppercase() {
                    ok = false;
                    break;
                }
                k += 1;
            }
            if ok {
                let mut m = k;
                while m < bytes.len()
                    && (bytes[m] == b' '
                        || bytes[m] == b'\t'
                        || bytes[m] == b'\n'
                        || bytes[m] == b'\r')
                {
                    m += 1;
                }
                if m < bytes.len() && bytes[m] == b'>' {
                    return Some(m + 1);
                }
            }
        }
        i += 1;
    }
    None
}

/// Remove chain-of-thought delimited by `<think>...</think>` (case-insensitive
/// tag names) from an LLM completion. Strips every occurrence; if a `<think>`
/// is opened but never closed, strips from the opening tag to the end of the
/// string.
///
/// Surviving content is preserved **verbatim** (including its original
/// newlines / markdown structure) — only the gap left by a removed block is
/// normalized to a single space when both neighbouring characters are
/// non-whitespace, and the result is trimmed. Never panics on odd input.
pub fn strip_cot(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut keep_ranges: Vec<(usize, usize)> = Vec::new();
    let mut cursor = 0usize;
    let mut last_keep = 0usize;

    while cursor < bytes.len() {
        if bytes[cursor] == b'<' {
            let mut after_open = 0usize;
            if is_open_tag(bytes, cursor, &mut after_open) {
                if cursor > last_keep {
                    keep_ranges.push((last_keep, cursor));
                }
                if let Some(after_close) = find_close_tag(bytes, after_open) {
                    cursor = after_close;
                    last_keep = after_close;
                    continue;
                }
                // unterminated: drop the rest of the input
                last_keep = bytes.len();
                break;
            }
        }
        cursor += 1;
    }
    if last_keep < bytes.len() {
        keep_ranges.push((last_keep, bytes.len()));
    }

    let mut out = String::with_capacity(input.len());
    for (idx, &(a, b)) in keep_ranges.iter().enumerate() {
        if idx > 0 {
            let prev_end = keep_ranges[idx - 1].1;
            let prev_ws = bytes[..prev_end]
                .last()
                .is_some_and(u8::is_ascii_whitespace);
            let next_ws = bytes[a..b].first().is_some_and(u8::is_ascii_whitespace);
            if !prev_ws && !next_ws {
                out.push(' ');
            }
        }
        out.push_str(&input[a..b]);
    }
    out.trim().to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::strip_cot;

    #[test]
    fn clean_text_unchanged() {
        let s = "The quick brown fox jumps.";
        assert_eq!(strip_cot(s), s);
    }

    #[test]
    fn single_block_removed() {
        let s = "answer<think>secret chain of thought</think>done";
        assert_eq!(strip_cot(s), "answer done");
    }

    #[test]
    fn multiple_blocks_removed() {
        let s = "a<think>one</think>b<think>two</think>c";
        assert_eq!(strip_cot(s), "a b c");
    }

    #[test]
    fn uppercase_and_nested_like_tags() {
        let s = "x<THINK>secret</think>y<think>more</think>z";
        assert_eq!(strip_cot(s), "x y z");
    }

    #[test]
    fn unterminated_trailing_stripped_to_eof() {
        let s = "hello<think>never closed";
        assert_eq!(strip_cot(s), "hello");
    }

    #[test]
    fn cot_with_newlines_inside_preserves_surrounding_newlines() {
        let s = "lead\n<think>\n internal\n reasoning\n</think>\ntrail";
        assert_eq!(strip_cot(s), "lead\n\ntrail");
    }

    #[test]
    fn markdown_summary_structure_preserved() {
        let s = "## Summary\n\nKey point.\n\n<think>hidden</think>## Details\n\nMore text.";
        assert_eq!(
            strip_cot(s),
            "## Summary\n\nKey point.\n\n## Details\n\nMore text."
        );
    }

    #[test]
    fn literal_word_think_preserved() {
        let s = "I think therefore I am, but no tags here.";
        assert_eq!(strip_cot(s), s);
    }

    #[test]
    fn partial_open_stripped() {
        let s = "hello<think> x";
        assert_eq!(strip_cot(s), "hello");
    }
}
