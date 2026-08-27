//! Shared LLM prompt construction for the `arags` client.
//!
//! Summarization prompts (file / module / project) are centralized here so the
//! three scopes emit a structurally identical article (same role sentence and
//! same mandated top-level sections) and only vary in scope-specific guidance.
//! The digest (query `-qa`) prompt is also centralized for homogeneity.

/// Scope of a summarization request.
#[derive(Clone, Copy, Debug)]
pub enum SummarizeScope {
    /// A single source file: its purpose, public API/signature, key behavior.
    File,
    /// A module spanning files: cross-file responsibilities and wiring.
    Module,
    /// The whole project: architecture, entrypoints, global invariants.
    Project,
}

/// Canonical role sentence, identical for every summarization scope.
const ROLE: &str = "You are a technical writer maintaining a project knowledge base.";

/// Trailing instruction mandating the fixed section layout, identical for every
/// scope (no extra preamble before the first section).
const SECTION_DIRECTIVE: &str = "Rewrite this into a clean, structured knowledge-base article. Use exactly these top-level sections, in this order, with no extra preamble:\n## Summary\n## Key Findings / Artifacts\n## Related";

/// Scope-specific guidance that varies the focus of the summary. This is the
/// only part of the prompt that depends on [`SummarizeScope`].
fn scope_guidance(scope: SummarizeScope) -> &'static str {
    match scope {
        SummarizeScope::File => {
            "Focus on this file's purpose, its public API/signature, and its key behaviors."
        }
        SummarizeScope::Module => {
            "Focus on this module's cross-file responsibilities and how its parts fit together."
        }
        SummarizeScope::Project => {
            "Below is an answer previously produced by a query-answer system, along with its provenance (source chunk ids and content hashes). Rewrite it into a clean, structured knowledge-base article."
        }
    }
}

/// Build the instruction for the summarizer, shared across file/module/project
/// scopes so output structure stays consistent.
///
/// * `scope` — which summarization scope is requested.
/// * `source` — a short label (file path, module name, or project name).
/// * `content` — the text to summarize (answer text, or extracted source).
/// * `provenance` — optional provenance metadata (chunk ids / hashes).
///
/// The prompt always opens with the canonical role sentence, embeds `source`,
/// `content`, and optional `provenance` in a deterministic layout, and ends with
/// the identical section directive. Only the scope guidance line differs.
#[must_use]
pub fn build_summarize_prompt(
    scope: SummarizeScope,
    source: &str,
    content: &str,
    provenance: Option<&str>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str(ROLE);
    prompt.push('\n');
    prompt.push('\n');
    prompt.push_str(scope_guidance(scope));
    prompt.push_str("\n\nSOURCE:\n");
    prompt.push_str(source);
    prompt.push_str("\n\nCONTENT:\n");
    prompt.push_str(content);
    if let Some(prov) = provenance {
        prompt.push_str("\n\nPROVENANCE:\n");
        prompt.push_str(prov);
    }
    prompt.push_str("\n\n");
    prompt.push_str(SECTION_DIRECTIVE);
    prompt
}

/// Build the client-side digest (query `-qa`) prompt from a question and the
/// retrieved project context. Centralized here so it is homogeneous with the
/// other client prompts.
#[must_use]
pub fn build_digest_prompt(question: &str, context: &str) -> String {
    format!(
        "Based on the following project context, answer this question concisely and with provenance:\n\nQuestion: {question}\n\nContext:\n{context}"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const ROLE_SENTENCE: &str = "You are a technical writer maintaining a project knowledge base.";
    const HEADERS: [&str; 3] = ["## Summary", "## Key Findings / Artifacts", "## Related"];

    #[test]
    fn all_scopes_contain_canonical_sections() {
        for scope in [
            SummarizeScope::File,
            SummarizeScope::Module,
            SummarizeScope::Project,
        ] {
            let prompt = build_summarize_prompt(scope, "src/foo.rs", "some content", None);
            assert!(prompt.contains(ROLE_SENTENCE), "missing role in {scope:?}");
            for header in HEADERS {
                assert!(prompt.contains(header), "missing {header} in {scope:?}");
            }
        }
    }

    #[test]
    fn scope_changes_guidance_only() {
        // Identical source/content/provenance → prompts share the canonical
        // prefix (role + mandated sections) and differ only in guidance.
        let file = build_summarize_prompt(SummarizeScope::File, "src/foo.rs", "c", Some("p"));
        let module = build_summarize_prompt(SummarizeScope::Module, "src/foo.rs", "c", Some("p"));
        let project = build_summarize_prompt(SummarizeScope::Project, "src/foo.rs", "c", Some("p"));

        // Common prefix: the canonical role sentence.
        assert!(file.starts_with(ROLE_SENTENCE));
        assert!(module.starts_with(ROLE_SENTENCE));
        assert!(project.starts_with(ROLE_SENTENCE));

        // Not identical to each other (guidance differs).
        assert_ne!(file, module);
        assert_ne!(module, project);
        assert_ne!(file, project);

        // Same set of section headers present in every scope.
        for header in HEADERS {
            assert!(file.contains(header));
            assert!(module.contains(header));
            assert!(project.contains(header));
        }
    }

    #[test]
    fn provenance_optional() {
        let without = build_summarize_prompt(SummarizeScope::Project, "src/foo.rs", "c", None);
        assert!(
            !without.contains("PROVENANCE:"),
            "None must omit PROVENANCE:"
        );

        let with = build_summarize_prompt(
            SummarizeScope::Project,
            "src/foo.rs",
            "c",
            Some("chunk-1:deadbeef"),
        );
        assert!(
            with.contains("PROVENANCE:\nchunk-1:deadbeef"),
            "Some must include PROVENANCE:"
        );
    }
}
