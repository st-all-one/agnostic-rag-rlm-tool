use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Prepare the repository: create `.arags.toml` and (by default) index it.
    Init {
        /// Run `arags index` after creating the config (default: true).
        #[arg(long)]
        index: bool,

        /// Skip running `arags index` after creating the config.
        #[arg(long, conflicts_with = "index")]
        no_index: bool,
    },

    /// Index a project directory (client streams raw file text to the server).
    Index {
        /// Directory to index.
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Ignore patterns (glob). Can be specified multiple times.
        #[arg(long = "ignore", action = clap::ArgAction::Append)]
        ignore_patterns: Vec<String>,

        /// Force-include patterns (glob) that bypass the default ignores.
        /// Can be specified multiple times.
        #[arg(long = "force-include", action = clap::ArgAction::Append)]
        force_include: Vec<String>,

        /// Register this project for background auto-update (like
        /// `git maintenance`): persists `[watch] enabled = true` in
        /// `.arags.toml` and starts a detached watcher daemon that re-indexes
        /// changed files after a 1-minute quiet window.
        #[arg(long)]
        register: bool,

        /// Stop the background watcher and remove the registration from
        /// `.arags.toml`.
        #[arg(long, conflicts_with = "register")]
        unregister: bool,
    },

    /// Search project with hybrid BM25 + semantic (server-side).
    Search {
        /// Search query.
        query: String,

        /// Top K results.
        #[arg(long, default_value_t = 10)]
        top_k: usize,

        /// File pattern filter.
        #[arg(long)]
        file_pattern: Option<String>,

        /// Minimum score threshold.
        #[arg(long)]
        min_score: Option<f32>,

        /// Search across all indexed projects.
        #[arg(short, long)]
        all: bool,

        /// Search tier: fts, entity, vector, summary, auto (default: auto).
        #[arg(long, default_value = "auto")]
        tier: String,

        /// Maximum tokens in output (0 = unlimited).
        #[arg(long, default_value_t = 8000)]
        max_tokens: usize,
    },

    /// (hidden) Background watch daemon; spawned by `index --register`.
    #[command(hide = true)]
    WatchDaemon {
        /// Project root to monitor.
        root: PathBuf,
    },

    /// Run as an RLM volunteer: claim summary jobs and synthesize them with
    /// your local LLM (configure in ~/.arags/arags.toml [volunteer]).
    Volunteer {
        /// Process at most one job, then exit.
        #[arg(long)]
        once: bool,
    },

    /// Query with on-demand QA: `-qa` digests via the user's LLM; `--cache-id`
    /// does a deterministic 1:1 lookup.
    Query {
        /// Question to analyze.
        question: String,

        /// LLM backend name (overrides config).
        #[arg(long)]
        backend: Option<String>,

        /// Model name (overrides config).
        #[arg(long)]
        model: Option<String>,

        /// Direct lookup of a previously served answer by stable cache id
        /// (plan 017, anti-drift; no re-digest, no re-index).
        #[arg(long)]
        cache_id: Option<String>,

        /// Use the semantic query-answer cache (QueryWithCache + client
        /// digest-once via the user's LLM).
        #[arg(long)]
        qa: bool,
    },

    /// Memory administration (admin-gated on the server): list / get / invalidate /
    /// cleanup cached query-answer memory.
    Memory {
        #[command(subcommand)]
        cmd: MemoryCmd,
    },

    /// Persist a served answer as a structured wiki page using the user's LLM.
    Persist {
        /// The `cache_id` (response id) emitted by `arags query -qa`.
        response_id: String,

        /// Optional title for the wiki page (defaults to a slug of the answer).
        #[arg(long)]
        title: Option<String>,
    },

    /// Show the current user's query history (server-scoped by refresh token).
    History {
        /// Limit results.
        #[arg(long, default_value_t = 20)]
        limit: usize,

        /// View another user's history (admin only; server enforces scope).
        #[arg(long)]
        user: Option<String>,
    },
}

/// Subcommands of `arags memory` (plan 019).
#[derive(Subcommand, Debug)]
pub enum MemoryCmd {
    /// List cached query/answer memory for a project.
    List {
        /// Project scope.
        #[arg(long)]
        project: Option<String>,

        /// Maximum number of entries.
        #[arg(long, default_value_t = 50)]
        limit: i64,

        /// Include entity information alongside entries.
        #[arg(long)]
        include_entities: bool,
    },
    /// Fetch a single cached answer by id (admin/debug).
    Get {
        /// Answer id.
        cache_id: String,
    },
    /// Invalidate cached answers (admin).
    Invalidate {
        /// Target answer id. When empty, purges the legacy result cache.
        #[arg(long)]
        cache_id: Option<String>,

        /// Project whose legacy result cache to purge.
        #[arg(long)]
        project: Option<String>,

        /// Hard delete instead of soft stale.
        #[arg(long)]
        delete: bool,

        /// Also invalidate nearby questions within this cosine radius.
        #[arg(long)]
        radius: Option<f32>,

        /// Reason for invalidation (audit).
        #[arg(long)]
        reason: Option<String>,
    },
    /// Run (or dry-run) cache cleanup / decay / consolidation.
    Cleanup {
        /// Dry run — report what would change without modifying.
        #[arg(long)]
        dry_run: bool,

        /// Project scope.
        #[arg(long)]
        project: Option<String>,
    },
}
