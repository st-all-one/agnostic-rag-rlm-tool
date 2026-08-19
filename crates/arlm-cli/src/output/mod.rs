pub mod json;
pub mod markdown;
pub mod prompt;
pub mod tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Tree,
    Markdown,
    Prompt,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Tree => write!(f, "tree"),
            Self::Markdown => write!(f, "markdown"),
            Self::Prompt => write!(f, "prompt"),
        }
    }
}

pub fn success(msg: &str) {
    let style = console::Style::new().green().bold();
    eprintln!("{} {}", style.apply_to("✓"), msg);
}

pub fn error(msg: &str) {
    let style = console::Style::new().red().bold();
    eprintln!("{} {}", style.apply_to("✗"), msg);
}

pub fn info(msg: &str) {
    let style = console::Style::new().cyan();
    eprintln!("{} {}", style.apply_to("→"), msg);
}

pub fn warn(msg: &str) {
    let style = console::Style::new().yellow();
    eprintln!("{} {}", style.apply_to("⚠"), msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_display() {
        assert_eq!(Format::Json.to_string(), "json");
        assert_eq!(Format::Tree.to_string(), "tree");
        assert_eq!(Format::Markdown.to_string(), "markdown");
        assert_eq!(Format::Prompt.to_string(), "prompt");
    }
}
