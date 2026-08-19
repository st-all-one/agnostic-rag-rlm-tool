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
        clippy::cast_precision_loss
    )
)]
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
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
    #[arg(short, long, global = true, default_value = "tree")]
    format: OutputFormatArg,

    /// Project path
    #[arg(short, long, global = true, default_value = ".")]
    project: PathBuf,
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
    },

    /// Index a project directory
    Index {
        /// Directory to index
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum tokens per chunk
        #[arg(long, default_value_t = 512)]
        chunk_size: usize,
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
    },

    /// Build context for an agent task
    Context {
        /// Task description
        task: String,

        /// Top K results
        #[arg(long, default_value_t = 10)]
        top_k: usize,
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
    },

    /// Manage sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },

    /// Consolidate memory (dedup, cleanup)
    Consolidate,

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

    let format = match cli.format {
        OutputFormatArg::Json => output::Format::Json,
        OutputFormatArg::Tree => output::Format::Tree,
        OutputFormatArg::Markdown => output::Format::Markdown,
        OutputFormatArg::Prompt => output::Format::Prompt,
    };

    let rt = tokio::runtime::Runtime::new()?;

    match cli.command {
        Commands::Run {
            task,
            llm,
            backend,
            model,
            depth,
            max_nodes,
            concurrency,
            max_budget,
            live,
        } => rt.block_on(commands::run::execute(commands::run::RunConfig {
            task: &task,
            llm,
            backend: backend.as_deref(),
            model: model.as_deref(),
            depth,
            max_nodes,
            concurrency,
            max_budget,
            project: &cli.project,
            format,
            verbose: cli.verbose,
            live,
        })),
        Commands::Index { path, chunk_size } => {
            commands::index::execute(commands::index::IndexConfig {
                path: &path,
                chunk_size,
                project: &cli.project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Search {
            query,
            top_k,
            file_pattern,
            min_score,
        } => commands::search::execute(commands::search::SearchConfig {
            query: &query,
            top_k,
            file_pattern: file_pattern.as_deref(),
            min_score,
            project: &cli.project,
            format,
            verbose: cli.verbose,
        }),
        Commands::Query {
            question,
            backend,
            model,
        } => rt.block_on(commands::query::execute(commands::query::QueryConfig {
            question: &question,
            backend: backend.as_deref(),
            model: model.as_deref(),
            project: &cli.project,
            format,
            verbose: cli.verbose,
        })),
        Commands::Context { task, top_k } => {
            commands::context::execute(commands::context::ContextConfig {
                task: &task,
                top_k,
                project: &cli.project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Status { run_id } => {
            commands::status::execute(run_id.as_deref(), &cli.project, format)
        }
        Commands::History { limit } => {
            commands::history::execute(commands::history::HistoryConfig {
                limit,
                project: &cli.project,
                format,
            })
        }
        Commands::Cost { run_id } => {
            commands::cost::execute(run_id.as_deref(), &cli.project, format);
            Ok(())
        }
        Commands::Session { action } => match action {
            SessionAction::Create { title } => {
                commands::session::execute_create(&title, &cli.project, format)
            }
            SessionAction::Resume { session_id } => {
                commands::session::execute_resume(&session_id, &cli.project, format)
            }
            SessionAction::List => commands::session::execute_list(&cli.project, format),
        },
        Commands::Consolidate => {
            commands::consolidate::execute(commands::consolidate::ConsolidateConfig {
                project: &cli.project,
                format,
                verbose: cli.verbose,
            })
        }
        Commands::Serve { port, host, mcp } => {
            rt.block_on(commands::serve::execute(commands::serve::ServeConfig {
                port,
                host: &host,
                project: &cli.project,
                verbose: cli.verbose,
                mcp,
            }))
        }
    }
}
