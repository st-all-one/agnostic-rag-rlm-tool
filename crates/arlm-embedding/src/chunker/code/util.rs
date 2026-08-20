//! Shared helpers for code chunking.

use crate::chunker::{RawChunk, estimate_tokens};

/// Merge consecutive small chunks that fit within `max_tokens`.
#[must_use]
pub fn merge_small_chunks<'a>(chunks: Vec<RawChunk<'a>>, max_tokens: usize) -> Vec<RawChunk<'a>> {
    if chunks.len() <= 1 {
        return chunks;
    }

    let mut merged = Vec::with_capacity(chunks.len());
    let mut pending: Option<RawChunk<'a>> = None;

    for chunk in chunks {
        match pending.take() {
            Some(prev) => {
                let combined_tokens =
                    estimate_tokens(&prev.content) + estimate_tokens(&chunk.content);
                if combined_tokens <= max_tokens
                    && prev.chunk_type == chunk.chunk_type
                    && prev.language == chunk.language
                {
                    let mut combined =
                        String::with_capacity(prev.content.len() + chunk.content.len());
                    combined.push_str(&prev.content);
                    combined.push_str(&chunk.content);
                    pending = Some(RawChunk {
                        offset_start: prev.offset_start,
                        offset_end: chunk.offset_end,
                        line_start: prev.line_start,
                        line_end: chunk.line_end,
                        content: std::borrow::Cow::Owned(combined),
                        language: chunk.language,
                        chunk_type: chunk.chunk_type,
                    });
                } else {
                    merged.push(prev);
                    pending = Some(chunk);
                }
            }
            None => {
                pending = Some(chunk);
            }
        }
    }

    if let Some(last) = pending {
        merged.push(last);
    }

    merged
}

/// Check if a trimmed line looks like the start of a code structure.
#[must_use]
pub fn is_structure_start(line: &str, language: Option<&str>) -> bool {
    match language {
        Some("rust") => {
            line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub(crate) fn ")
                || line.starts_with("async fn ")
                || line.starts_with("pub async fn ")
                || line.starts_with("struct ")
                || line.starts_with("pub struct ")
                || line.starts_with("enum ")
                || line.starts_with("pub enum ")
                || line.starts_with("impl ")
                || line.starts_with("trait ")
                || line.starts_with("pub trait ")
                || line.starts_with("mod ")
                || line.starts_with("pub mod ")
                || line.starts_with("type ")
        }
        Some("python") => {
            line.starts_with("def ") || line.starts_with("class ") || line.starts_with("async def ")
        }
        Some("javascript" | "typescript") => {
            line.starts_with("function ")
                || line.starts_with("export function ")
                || line.starts_with("export default function ")
                || line.starts_with("class ")
                || line.starts_with("export class ")
                || line.starts_with("async function ")
                || line.starts_with("export async function ")
                || line.starts_with("const ")
                || line.starts_with("let ")
        }
        Some("go") => {
            line.starts_with("func ") || line.starts_with("type ") || line.starts_with("package ")
        }
        Some("java") => {
            line.starts_with("public class ")
                || line.starts_with("class ")
                || line.starts_with("public interface ")
                || line.starts_with("interface ")
                || line.starts_with("public enum ")
                || line.starts_with("enum ")
                || (line.contains("public static void main") && line.contains('('))
        }
        Some("cpp" | "c") => {
            line.starts_with("void ")
                || line.starts_with("int ")
                || line.starts_with("static ")
                || line.starts_with("extern ")
                || line.starts_with("class ")
                || line.starts_with("struct ")
                || line.starts_with("namespace ")
                || line.starts_with("template")
        }
        _ => false,
    }
}

/// Find the 1-based line number for a byte offset.
#[must_use]
pub(crate) fn byte_start_line(content: &str, byte_offset: usize) -> usize {
    content[..byte_offset]
        .chars()
        .filter(|&c| c == '\n')
        .count()
        + 1
}
