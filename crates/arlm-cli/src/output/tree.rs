use std::fmt::Write as _;

use console::Style;

pub fn render_tree(root_id: &str, task: &str, max_depth: u32) -> String {
    let bold = Style::new().bold();
    let dim = Style::new().dim();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        bold.apply_to(format!("RLM run {root_id} (maxDepth={max_depth})"))
    );
    let _ = writeln!(
        out,
        "{}",
        dim.apply_to(format!("└─ [completed/solve] {task} ✓"))
    );
    out
}

pub fn render_search_results(results: &[SearchResultItem]) -> String {
    let bold = Style::new().bold();
    let dim = Style::new().dim();
    let cyan = Style::new().cyan();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}",
        bold.apply_to(format!("Search Results ({})", results.len()))
    );

    for (i, r) in results.iter().enumerate() {
        let prefix = if i == results.len() - 1 {
            "└─"
        } else {
            "├─"
        };
        let score_str = format!("{:.2}", r.score);
        let score_display = if r.score >= 0.8 {
            console::Style::new().green().apply_to(score_str)
        } else if r.score >= 0.5 {
            console::Style::new().yellow().apply_to(score_str)
        } else {
            console::Style::new().dim().apply_to(score_str)
        };
        let _ = writeln!(
            out,
            "{} {} {} (score: {})",
            prefix,
            cyan.apply_to(&r.file_path),
            dim.apply_to(format!("{}:{}", r.line_start, r.line_end)),
            score_display,
        );
    }
    out
}

pub fn render_history_table(rows: &[HistoryRow]) -> String {
    let bold = Style::new().bold();
    let header = format!(
        "{:<20} {:<40} {:<10} {}",
        "DATE", "QUERY", "DURATION", "RESULTS"
    );
    let mut out = String::new();
    let _ = writeln!(out, "{}", bold.apply_to(header));
    let _ = writeln!(out, "{}", "-".repeat(80));

    for r in rows {
        let query_display = if r.query.len() > 37 {
            format!("{}...", &r.query[..37])
        } else {
            r.query.clone()
        };
        let _ = writeln!(
            out,
            "{:<20} {:<40} {:<10} {}",
            r.date, query_display, r.duration, r.results,
        );
    }
    out
}

pub struct SearchResultItem {
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub score: f32,
}

pub struct HistoryRow {
    pub date: String,
    pub query: String,
    pub duration: String,
    pub results: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_tree() {
        let tree = render_tree("run-001", "analyze project", 3);
        assert!(tree.contains("run-001"));
        assert!(tree.contains("analyze project"));
    }

    #[test]
    fn test_render_search_results_empty() {
        let results = render_search_results(&[]);
        assert!(results.contains("Search Results (0)"));
    }

    #[test]
    fn test_render_history_table() {
        let rows = vec![HistoryRow {
            date: "2024-01-15 10:30".into(),
            query: "find bugs".into(),
            duration: "2.3s".into(),
            results: "5".into(),
        }];
        let table = render_history_table(&rows);
        assert!(table.contains("find bugs"));
    }
}
