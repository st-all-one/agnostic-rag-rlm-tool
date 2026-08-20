use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run RLM recursively on a task (requires --llm)
    Run {
        /// Task description
        task: String,

        /// Enable LLM mode (required for run)
        #[arg(long)]
        llm: bool,

        /// LLM backend: openai, anthropic, ollama, gemini
        #[arg(long)]
        backend: Option<String>,

        /// Model name
        #[arg(long)]
        model: Option<String>,

        /// Maximum recursion depth
        #[arg(long, default_value_t = 3)]
        depth: u32,

        /// Maximum number of nodes
        #[arg(long, default_value_t = 50)]
        max_nodes: u32,

        /// Concurrency limit
        #[arg(short = 'j', long, default_value_t = 4)]
        concurrency: usize,

        /// Maximum budget in USD
        #[arg(long, default_value_t = 1.0)]
        max_budget: f64,

        /// Render the RLM recursion tree in real time
        #[arg(long)]
        live: bool,

        /// Agent identifier for cost attribution (or set `ARLM_AGENT` env var)
        #[arg(long, env = "ARLM_AGENT")]
        agent: Option<String>,

        /// Custom tool available to the solver. Format: "name:description" or "name:param1,param2:description"
        /// Can be specified multiple times.
        #[arg(long = "tool", action = clap::ArgAction::Append)]
        tools: Vec<String>,

        /// Session ID for persistent multi-turn context
        #[arg(long)]
        session: Option<String>,

        /// Run in REPL mode: LLM generates code blocks that are executed in a loop
        #[arg(long)]
        repl: bool,

        /// Persist the run output as a wiki page
        #[arg(long)]
        persist: bool,
    },

    /// Index a project directory
    Index {
        /// Directory to index
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum tokens per chunk
        #[arg(long, default_value_t = 512)]
        chunk_size: usize,

        /// Ignore patterns (glob). Can be specified multiple times.
        /// Default: .env, .env.*, *.pem, *.key
        #[arg(long = "ignore", action = clap::ArgAction::Append)]
        ignore_patterns: Vec<String>,

        /// Watch for file changes and reindex automatically
        #[arg(short, long)]
        watch: bool,
    },

    /// Search project with hybrid BM25 + semantic
    Search {
        /// Search query
        query: String,

        /// Top K results
        #[arg(long, default_value_t = 10)]
        top_k: usize,

        /// File pattern filter
        #[arg(long)]
        file_pattern: Option<String>,

        /// Minimum score threshold
        #[arg(long)]
        min_score: Option<f32>,

        /// Search across all indexed projects
        #[arg(short, long)]
        all: bool,

        /// Search tier: fts, entity, vector, auto (default: auto)
        #[arg(long, default_value = "auto")]
        tier: String,

        /// Maximum tokens in output (0 = unlimited)
        #[arg(long, default_value_t = 8000)]
        max_tokens: u32,

        /// Persist the search output as a wiki page
        #[arg(long)]
        persist: bool,
    },

    /// Query with RLM analysis
    Query {
        /// Question to analyze
        question: String,

        /// LLM backend
        #[arg(long)]
        backend: Option<String>,

        /// Model name
        #[arg(long)]
        model: Option<String>,

        /// Use RLM engine for recursive analysis
        #[arg(long)]
        llm: bool,
    },

    /// Build context for an agent task
    Context {
        /// Task description
        task: String,

        /// Top K results
        #[arg(long, default_value_t = 10)]
        top_k: usize,

        /// Search across all indexed projects
        #[arg(short, long)]
        all: bool,

        /// Search tier: fts, entity, vector, auto (default: auto)
        #[arg(long, default_value = "auto")]
        tier: String,

        /// Maximum tokens in output (0 = unlimited)
        #[arg(long, default_value_t = 8000)]
        max_tokens: u32,

        /// Persist the context output as a wiki page
        #[arg(long)]
        persist: bool,
    },

    /// Show project and run status
    Status {
        /// Specific run ID to check
        run_id: Option<String>,
    },

    /// Show query history
    History {
        /// Limit results
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Show cost summary
    Cost {
        /// Show costs for a specific run
        run_id: Option<String>,

        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
    },

    /// Manage sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Consolidate memory (dedup, cleanup)
    Consolidate,

    /// Run salience decay on indexed chunks
    Decay {
        /// Dry run — show what would be decayed without modifying
        #[arg(long)]
        dry_run: bool,
    },

    /// Cancel a running RLM run
    Cancel {
        /// Run ID to cancel
        run_id: String,
    },

    /// List checkpoints for runs
    Checkpoints {
        /// Specific run ID to check
        run_id: Option<String>,
    },

    /// Restore a persisted wiki page
    RestorePage {
        /// Page name to restore
        page_name: String,
    },

    /// Manage wiki with git integration
    Wiki {
        /// Action: init, commit, log
        action: String,
    },

    /// Search for entities in indexed code
    Entities {
        /// Entity query
        query: String,
    },

    /// Persist search/analysis results as wiki pages
    Persist {
        /// Title for the wiki page
        #[arg(long)]
        title: Option<String>,

        /// Query that produced this result
        #[arg(long)]
        query: Option<String>,
    },

    /// Start HTTP API server
    Serve {
        /// Port to listen on
        #[arg(long, default_value_t = 8080)]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Enable MCP (Model Context Protocol) server on /mcp endpoint
        #[arg(long)]
        mcp: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum SessionAction {
    /// Create a new session
    Create {
        /// Session title
        title: String,
    },
    /// Resume an existing session
    Resume {
        /// Session ID
        session_id: String,
    },
    /// List all sessions
    List,
}
