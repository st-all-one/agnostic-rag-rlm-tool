use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Prepare the repository: create `.arags.toml` (a real bootstrap utility)
    /// and optionally register a watch daemon / index.
    Init {
        /// Canonical project name (knowledge entity). Required: the project is
        /// a conceptual entity shared across worktrees, NOT derived from the
        /// path. If omitted on a TTY you will be prompted; in `--non-interactive`
        /// mode it is required and an error is raised when absent.
        #[arg(long)]
        name: Option<String>,

        /// Ignore patterns (glob). Can be specified multiple times. When
        /// omitted, patterns are seeded from `.gitignore` (or existing config).
        #[arg(long = "ignore", action = clap::ArgAction::Append)]
        ignore: Vec<String>,

        /// Local server-address override written to `.arags.toml` `[server]`.
        #[arg(long = "server-addr")]
        server_addr: Option<String>,

        /// Register the background watch daemon now (like `index --register`).
        #[arg(long)]
        register: bool,

        /// Do NOT register the background watch daemon (default).
        #[arg(long, conflicts_with = "register")]
        no_register: bool,

        /// Run `arags index` after writing the config (default: true).
        #[arg(long)]
        index: bool,

        /// Skip running `arags index` after writing the config.
        #[arg(long, conflicts_with = "index")]
        no_index: bool,

        /// Never prompt; fail if any required value (e.g. `--name`) is missing.
        #[arg(long)]
        non_interactive: bool,
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

    /// Search project with hybrid BM25 + semantic (server-side, OBJECTIVE).
    ///
    /// Pure retrieval over chunks. When RLM Summaries / Exploration Maps are
    /// sufficiently close in vector space they are returned too (the "unified
    /// query" behavior). This command NEVER invokes the user's LLM — it is a
    /// pure data-plane retrieve. For the no-LLM server-side context builder
    /// (the old `query` default), use `--context`.
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

        /// Return server-side context (BuildContext RPC) instead of the search
        /// results. Objective-only: NO client LLM is invoked. This is the
        /// migration target for the old no-LLM `query` default path.
        #[arg(long)]
        context: bool,

        /// Time-travel (plan 021): return only knowledge revisions active at
        /// this unix-second epoch. 0 / unset = live state.
        #[arg(long)]
        as_of_epoch: Option<i64>,

        /// Time-travel alias for `--as-of-epoch`: an RFC3339 timestamp converted
        /// to seconds before sending. Conflicts with `--as-of-epoch`.
        #[arg(long, conflicts_with = "as_of_epoch")]
        as_of: Option<String>,
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

    /// [DEPRECATED] Use `arags ask` instead.
    ///
    /// This alias prints a deprecation warning and routes to the same `ask`
    /// logic (LLM digest is now implicit by default; `--cache-id` does a
    /// deterministic 1:1 lookup). The old no-LLM default context path has
    /// moved to `arags search --context`. Kept for ONE release for compat.
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

        /// Ignored (retained for shape compat). LLM digest is now implicit in
        /// `ask`; this flag is accepted but no longer required.
        #[arg(long)]
        qa: bool,

        /// Time-travel (plan 021): serve the cached answer revision active at
        /// this unix-second epoch. 0 / unset = live state.
        #[arg(long)]
        as_of_epoch: Option<i64>,

        /// Time-travel alias for `--as-of-epoch`: an RFC3339 timestamp converted
        /// to seconds before sending. Conflicts with `--as-of-epoch`.
        #[arg(long, conflicts_with = "as_of_epoch")]
        as_of: Option<String>,
    },

    /// Ask the user's local LLM to digest a question over the index.
    ///
    /// The LLM digest is IMPLICIT — every `ask` digests via the user's local
    /// LLM by default (this replaces the old `query -qa`). Deterministic
    /// lookup stays available: `--cache-id <id>` returns the cached answer
    /// with NO LLM call. Objective, LLM-free retrieval lives in `search`.
    Ask {
        /// Question to analyze.
        question: String,

        /// LLM backend name (overrides config).
        #[arg(long)]
        backend: Option<String>,

        /// Model name (overrides config).
        #[arg(long)]
        model: Option<String>,

        /// Direct lookup of a previously served answer by stable cache id
        /// (plan 017, anti-drift; no re-digest, no re-index). Overrides the
        /// LLM digest path and returns the cached answer deterministically.
        #[arg(long)]
        cache_id: Option<String>,

        /// Time-travel (plan 021): serve the cached answer revision active at
        /// this unix-second epoch. 0 / unset = live state.
        #[arg(long)]
        as_of_epoch: Option<i64>,

        /// Time-travel alias for `--as-of-epoch`: an RFC3339 timestamp converted
        /// to seconds before sending. Conflicts with `--as-of-epoch`.
        #[arg(long, conflicts_with = "as_of_epoch")]
        as_of: Option<String>,
    },

    /// Server maintenance administration (admin-gated on the server): list /
    /// get / invalidate / cleanup cached query-answer memory.
    #[command(name = "maintenance")]
    Maintenance {
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

    /// Explore persisted agent exploration maps (plan 022).
    Explore {
        #[command(subcommand)]
        cmd: ExploreCmd,
    },
}

/// Subcommands of `arags explore` (plan 022).
#[derive(Subcommand, Debug)]
pub enum ExploreCmd {
    /// Search exploration maps semantically before re-exploring from zero.
    Search {
        /// Free-text query (what are you about to investigate?).
        query: String,

        /// Project scope — the canonical project name (defaults to the
        /// current project's `[project].name`).
        #[arg(long)]
        project: Option<String>,

        /// Maximum number of maps.
        #[arg(long, default_value_t = 5)]
        limit: i32,

        /// Include stale maps (useful as history/archaeology).
        #[arg(long)]
        include_stale: bool,

        /// Time-travel (plan 021): surface the exploration revision active at
        /// this unix-second epoch (compared against epoch_created). 0 / unset =
        /// live state.
        #[arg(long)]
        as_of_epoch: Option<i64>,

        /// Time-travel alias for `--as-of-epoch`: an RFC3339 timestamp converted
        /// to seconds before sending. Conflicts with `--as-of-epoch`.
        #[arg(long, conflicts_with = "as_of_epoch")]
        as_of: Option<String>,
    },

    /// Persist an exploration map following the EXPLORATIONS.md contract.
    Persist {
        /// Markdown contract file (`-` reads stdin).
        #[arg(long)]
        map: PathBuf,

        /// Extra cited paths appended to the contract's `files:` header.
        #[arg(long = "paths", value_delimiter = ',')]
        paths: Vec<String>,
    },
}

/// Subcommands of `arags maintenance` (plan 019).
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

/// Resolve a time-travel request into a unix-second epoch.
///
/// Precedence: an explicit `--as-of-epoch` wins; otherwise an RFC3339
/// `--as-of` timestamp is parsed; otherwise `0` (live state).
///
/// # Errors
///
/// Returns an error if `--as-of` is present but not a valid RFC3339 timestamp.
pub fn resolve_as_of_epoch(as_of_epoch: Option<i64>, as_of: Option<String>) -> anyhow::Result<i64> {
    if let Some(epoch) = as_of_epoch {
        return Ok(epoch);
    }
    if let Some(ts) = as_of {
        let dt = chrono::DateTime::parse_from_rfc3339(&ts)
            .map_err(|e| anyhow::anyhow!("invalid --as-of timestamp {ts:?}: {e}"))?;
        return Ok(dt.timestamp());
    }
    Ok(0)
}
