use std::time::Instant;

use tracing::debug;

use crate::output::live_tree::model::LiveTree;

impl LiveTree {
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn render(&self) -> String {
        let start = Instant::now();
        let mut lines = Vec::new();
        if let Some(root_id) = &self.root_id {
            self.render_node(root_id, &mut lines, "", true, true);
        }
        let out = lines.join("\n");
        debug!(
            nodes = self.nodes.len(),
            elapsed_ms = start.elapsed().as_millis() as u64,
            "rendered live tree"
        );
        out
    }

    fn render_node(
        &self,
        id: &str,
        lines: &mut Vec<String>,
        prefix: &str,
        is_last: bool,
        is_root: bool,
    ) {
        if let Some(node) = self.nodes.get(id) {
            let icon = match node.status.as_str() {
                "complete" => "\u{2713}",
                "planning" | "running" => "\u{2026}",
                "solving" => "\u{00b7}",
                "failed" => "\u{2717}",
                "cancelled" => "\u{2298}",
                _ => "?",
            };

            let duration_str = if node.duration_ms > 0 {
                format!(" {}ms", node.duration_ms)
            } else {
                String::new()
            };

            let cost_str = if node.cost > 0.0 {
                format!(" ${:.4}", node.cost)
            } else {
                String::new()
            };

            let task_display = if node.task.len() > 60 {
                format!("{}...", &node.task[..57])
            } else {
                node.task.clone()
            };

            let connector = if is_root {
                ""
            } else if is_last {
                "\u{2514}\u{2500} "
            } else {
                "\u{251c}\u{2500} "
            };

            lines.push(format!(
                "{prefix}{connector}{icon} {id} (d{depth}) {task}{dur}{cost}",
                prefix = prefix,
                connector = connector,
                icon = icon,
                id = node.id,
                depth = node.depth,
                task = task_display,
                dur = duration_str,
                cost = cost_str,
            ));

            let child_prefix = if is_root {
                String::new()
            } else if is_last {
                format!("{prefix}    ")
            } else {
                format!("{prefix}\u{2502}   ")
            };

            let children: Vec<&str> = self
                .nodes
                .values()
                .filter(|n| n.parent.as_deref() == Some(id))
                .map(|n| n.id.as_str())
                .collect();

            for (i, child_id) in children.iter().enumerate() {
                self.render_node(
                    child_id,
                    lines,
                    &child_prefix,
                    i == children.len() - 1,
                    false,
                );
            }
        }
    }
}
