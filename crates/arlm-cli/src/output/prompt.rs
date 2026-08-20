use std::fmt::Write as _;

#[must_use]
pub fn render_search_context(results: &[PromptItem]) -> String {
    let mut ctx = String::from("## Project Context\n\n");

    for (i, r) in results.iter().enumerate() {
        let lang = r.language.as_deref().unwrap_or("");
        let _ = write!(
            ctx,
            "### File {} (score: {:.2})\n{}\n```{}\n{}\n```\n\n",
            i + 1,
            r.score,
            r.file_path,
            lang,
            r.content,
        );
    }

    ctx
}

pub struct PromptItem {
    pub file_path: String,
    pub score: f32,
    pub content: String,
    pub language: Option<String>,
}
