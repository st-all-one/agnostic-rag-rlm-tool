use std::fmt::Write as _;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_search_context() {
        let items = vec![PromptItem {
            file_path: "src/main.rs".into(),
            score: 0.85,
            content: "fn main() {}".into(),
            language: Some("rust".into()),
        }];
        let ctx = render_search_context(&items);
        assert!(ctx.contains("## Project Context"));
        assert!(ctx.contains("src/main.rs"));
    }
}
