pub mod json;
pub mod jsonl;
pub mod live_tree;
pub mod markdown;
pub mod prompt;
pub mod tree;

pub use live_tree::LiveTree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    FullJson,
    Path,
    Markdown,
    Text,
    Jsonl,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FullJson => write!(f, "full_json"),
            Self::Path => write!(f, "path"),
            Self::Markdown => write!(f, "markdown"),
            Self::Text => write!(f, "text"),
            Self::Jsonl => write!(f, "jsonl"),
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
