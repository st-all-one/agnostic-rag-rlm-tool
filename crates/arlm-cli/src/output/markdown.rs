use std::fmt::Write as _;

pub fn render_search_results(results: &[SuperItem]) -> String {
    let mut md = String::from("# Search Results\n\n");

    for (i, r) in results.iter().enumerate() {
        let lang = r.language.as_deref().unwrap_or("");
        let _ = writeln!(
            md,
            "## {} {} (score: {:.2})\n\n```{}\n{}\n```\n",
            i + 1,
            r.file_path,
            r.score,
            lang,
            r.content,
        );
    }

    md
}

pub fn render_run_result(task: &str, output: &str, duration_ms: u64) -> String {
    let mut md = String::new();
    let _ = writeln!(md, "# RLM Analysis\n");
    let _ = writeln!(md, "**Task:** {task}\n");
    let _ = writeln!(md, "**Duration:** {duration_ms}ms\n");
    let _ = writeln!(md, "## Result\n");
    let _ = writeln!(md, "{output}");
    md
}

pub struct SuperItem {
    pub file_path: String,
    pub score: f32,
    pub content: String,
    pub language: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_search_results() {
        let items = vec![SuperItem {
            file_path: "src/main.rs".into(),
            score: 0.9,
            content: "fn main() {}".into(),
            language: Some("rust".into()),
        }];
        let md = render_search_results(&items);
        assert!(md.contains("# Search Results"));
        assert!(md.contains("src/main.rs"));
    }

    #[test]
    fn test_render_run_result() {
        let md = render_run_result("analyze code", "found 3 issues", 1234);
        assert!(md.contains("analyze code"));
        assert!(md.contains("found 3 issues"));
    }
}
