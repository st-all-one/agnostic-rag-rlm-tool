#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        unsafe_code
    )
)]
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod config;
mod metrics;
mod output;
mod util;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(
    name = "arlm",
    about = "Agnostic RLM — Recursive Language Model CLI",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output with structured logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format: json, tree, markdown, prompt
    #[arg(short, long, global = true)]
    format: Option<OutputFormatArg>,

    /// Project path
    #[arg(short, long, global = true)]
    project: Option<PathBuf>,

    /// Config file path (default: ~/.arlm/config.toml)
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// LLM backend (overrides config)
    #[arg(long, global = true)]
    backend: Option<String>,

    /// Model name (overrides config)
    #[arg(long, global = true)]
    model: Option<String>,

    /// Agent name (overrides config)
    #[arg(long, global = true)]
    agent: Option<String>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormatArg {
    Json,
    Tree,
    Markdown,
    Prompt,
}

#[derive(Subcommand)]
enum Commands {
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

        /// Agent identifier for cost attribution (or set ARLM_AGENT env var)
        #[arg(long, env = "ARLM_AGENT")]
        agent: Option<String>,
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

#[derive(Subcommand)]
enum SessionAction {
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

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    let cli = Cli::parse();

    arlm_core::logging::init_logging(cli.verbose);

    // Load config file and merge with CLI args
    let cfg = if let Some(ref config_path) = cli.config {
        config::Config::load_from(config_path)?
    } else {
        config::Config::load().unwrap_or_default()
    };

    // Resolve final values: CLI args override config, config overrides defaults
    let project = cli
        .project
        .or(cfg.project)
        .unwrap_or_else(|| PathBuf::from("."));
    let backend = cli.backend.or(cfg.backend);
    let model = cli.model.or(cfg.model);
    let agent_name = cli.agent.or(cfg.agent.name);
    let format = cli
        .format
        .map(|f| match f {
            OutputFormatArg::Json => output::Format::Json,
            OutputFormatArg::Tree => output::Format::Tree,
            OutputFormatArg::Markdown => output::Format::Markdown,
            OutputFormatArg::Prompt => output::Format::Prompt,
        })
        .or_else(|| {
            cfg.format.as_deref().map(|s| match s {
                "json" => output::Format::Json,
                "tree" => output::Format::Tree,
                "markdown" => output::Format::Markdown,
                "prompt" => output::Format::Prompt,
                _ => output::Format::Tree,
            })
        })
        .unwrap_or(output::Format::Tree);

    let rt = tokio::runtime::Runtime::new()?;

    match cli.command {
        Commands::Run {
            task,
            llm,
            backend: cmd_backend,
            model: cmd_model,
            depth,
            max_nodes,
            concurrency,
            max_budget,
            live,
            agent: cmd_agent,
        } => rt.block_on(commands::run::execute(commands::run::RunConfig {
            task: &task,
            llm,
            backend: cmd_backend.as_deref().or(backend.as_deref()),
            model: cmd_model.as_deref().or(model.as_deref()),
            depth,
            max_nodes,
            concurrency,
            max_budget,
            project: &project,
            format,
            verbose: cli.verbose,
            live,
            agent: cmd_agent.as_deref().or(agent_name.as_deref()),
        })),
        Commands::Index { path, chunk_size, ignore_patterns, watch } => {
            commands::index::execute(commands::index::IndexConfig {
                path: &path,
                chunk_size,
                ignore_patterns: &ignore_patterns,
                watch,
                project: &project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Search {
            query,
            top_k,
            file_pattern,
            min_score,
            all,
            tier,
            max_tokens,
        } => rt.block_on(commands::search::execute(commands::search::SearchConfig {
            query: &query,
            top_k: cfg.search.top_k.unwrap_or(top_k as u32) as usize,
            file_pattern: file_pattern.as_deref(),
            min_score,
            all,
            tier: &tier,
            max_tokens: if max_tokens == 0 { cfg.search.max_tokens } else { Some(max_tokens) },
            project: &project,
            format,
            verbose: cli.verbose,
        })),
        Commands::Query {
            question,
            backend: cmd_backend,
            model: cmd_model,
            llm,
        } => rt.block_on(commands::query::execute(commands::query::QueryConfig {
            question: &question,
            backend: cmd_backend.as_deref().or(backend.as_deref()),
            model: cmd_model.as_deref().or(model.as_deref()),
            project: &project,
            format,
            verbose: cli.verbose,
            llm,
        })),
        Commands::Context { task, top_k, all, tier, max_tokens } => {
            rt.block_on(commands::context::execute(commands::context::ContextConfig {
                task: &task,
                top_k: cfg.search.top_k.unwrap_or(top_k as u32) as usize,
                all,
                tier: &tier,
                max_tokens: if max_tokens == 0 { cfg.search.max_tokens } else { Some(max_tokens) },
                project: &project,
                format,
                verbose: cli.verbose,
            }))
        }
        Commands::Status { run_id } => {
            commands::status::execute(run_id.as_deref(), &project, format)
        }
        Commands::History { limit } => {
            commands::history::execute(commands::history::HistoryConfig {
                limit,
                project: &project,
                format,
            })
        }
        Commands::Cost { run_id, agent: cmd_agent } => {
            commands::cost::execute(run_id.as_deref(), cmd_agent.as_deref().or(agent_name.as_deref()), &project, format)
        }
        Commands::Session { action } => match action {
            SessionAction::Create { title } => {
                commands::session::execute_create(&title, &project, format)
            }
            SessionAction::Resume { session_id } => {
                commands::session::execute_resume(&session_id, &project, format)
            }
            SessionAction::List => commands::session::execute_list(&project, format),
        },
        Commands::Consolidate => {
            commands::consolidate::execute(commands::consolidate::ConsolidateConfig {
                project: &project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Decay { dry_run } => commands::decay::execute(commands::decay::DecayArgs {
            dry_run,
            project: &project,
            format,
        }),
        Commands::Cancel { run_id } => {
            commands::cancel::execute(&run_id, &project, format)
        }
        Commands::Checkpoints { run_id } => {
            commands::checkpoints::execute(run_id.as_deref(), format)
        }
        Commands::RestorePage { page_name } => {
            commands::restore_page::execute(&page_name, &project, format)
        }
        Commands::Wiki { action } => {
            commands::wiki::execute(&action, &project, format)
        }
        Commands::Entities { query } => {
            commands::entities::execute(&query, &project, format)
        }
        Commands::Persist { title, query } => {
            commands::persist::execute(commands::persist::PersistArgs {
                title,
                query,
                project: &project,
                format,
            })
        }
        Commands::Serve { port, host, mcp } => {
            rt.block_on(commands::serve::execute(commands::serve::ServeConfig {
                port,
                host: &host,
                project: &project,
                verbose: cli.verbose,
                mcp,
            }))
        }
    }
}
