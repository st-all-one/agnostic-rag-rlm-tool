//! Internal admin CLI (container-only).
//!
//! Manages refresh tokens by opening [`Storage`] directly — **not** over gRPC,
//! so there is no remote privilege-escalation path. Only reachable from inside
//! the server container where the DB file is accessible.
//!
//! Subcommands:
//! - `create-refresh --username <u> --role <admin|non_admin>` → prints a new
//!   refresh token (plaintext, once).
//! - `revoke --id <id>` (or `--username <u>`) → revokes a refresh token.
//! - `prune-tokens --yes` → revokes **all** tokens (emergency response).

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use arags_storage::Storage;
use arags_storage::tokens::{self, NewToken, Role};

use crate::config::ServerConfig;
use crate::maintenance;

/// Internal token-management CLI.
#[derive(Parser)]
#[command(
    name = "admin",
    about = "Internal refresh-token management (container only)"
)]
pub struct AdminCli {
    #[command(subcommand)]
    pub command: AdminCommand,
}

/// Admin subcommands.
#[derive(Subcommand)]
pub enum AdminCommand {
    /// Create a refresh token and print its plaintext (once).
    CreateRefresh {
        /// Owning username (for audit).
        #[arg(long)]
        username: String,
        /// Role: `admin` or `non_admin`.
        #[arg(long, value_parser = parse_role)]
        role: Role,
    },
    /// Revoke a refresh token by id or username.
    Revoke {
        /// Token id to revoke.
        #[arg(long)]
        id: Option<String>,
        /// Revoke all tokens for this username.
        #[arg(long)]
        username: Option<String>,
    },
    /// Revoke every refresh token. Requires `--yes`.
    PruneTokens {
        /// Confirm the destructive prune.
        #[arg(long)]
        yes: bool,
    },
    /// Run server-side memory maintenance (consolidate + decay) directly
    /// against the DB, without going through gRPC. Mirrors the `TriggerMaintenance`
    /// RPC but is reachable only from inside the container.
    Consolidate {
        /// Project to maintain (empty = all projects).
        #[arg(long)]
        project: Option<String>,
        /// Compute the report without deleting anything.
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_role(s: &str) -> Result<Role, String> {
    s.parse::<Role>().map_err(|e| e.to_string())
}

/// Run the admin CLI with `env::args()` (the leading `admin` is already
/// consumed by the binary dispatcher).
///
/// # Errors
///
/// Returns an error on invalid arguments, a storage failure, or a refused
/// destructive operation.
pub async fn run() -> Result<()> {
    let args = std::iter::once("arags-server-admin".to_string()).chain(std::env::args().skip(2));
    let cli = AdminCli::parse_from(args);
    let config = ServerConfig::load().context("failed to load server config")?;
    let storage = Storage::open(&config.data_dir).context("failed to open storage")?;

    match cli.command {
        AdminCommand::CreateRefresh { username, role } => {
            let (id, plaintext) = tokens::create_token(
                &storage,
                &NewToken {
                    username: username.clone(),
                    role,
                    created_by: "cli".to_string(),
                },
            )?;
            println!("Token ID : {id}");
            println!("Username : {username}");
            println!("Role     : {role}");
            println!();
            println!("Refresh token (paste into client ~/.arags/arags.toml [auth].refresh_token):");
            println!("{plaintext}");
            eprintln!("WARNING: this token grants access for 1 year. Store it securely (0600).");
        }
        AdminCommand::Revoke { id, username } => {
            let revoked = match (id, username) {
                (Some(id), _) => tokens::revoke_token_by_id(&storage, &id, "cli")?,
                (None, Some(u)) => tokens::revoke_token_by_username(&storage, &u, "cli")?,
                (None, None) => bail!("specify --id or --username"),
            };
            if revoked {
                println!("Token revoked.");
            } else {
                println!("No matching (non-revoked) token found.");
            }
        }
        AdminCommand::PruneTokens { yes } => {
            if !yes {
                bail!("refusing to prune all tokens without --yes");
            }
            let n = tokens::revoke_all_tokens(&storage, "cli")?;
            println!("Pruned {n} token(s); all sessions invalidated.");
        }
        AdminCommand::Consolidate { project, dry_run } => {
            let project = project.unwrap_or_default();
            let scope = if project.is_empty() {
                "<all>".to_string()
            } else {
                project.clone()
            };
            let floor = config.maintenance.decay_score_floor;
            // Open the chunk vector store so decay/consolidate can also purge
            // orphan vectors (issue `agnostic-rlm-rs-fa25`). On a fresh store
            // usearch restores the on-disk dimensionality; a missing file falls
            // back to the default 384. `main` already runs a tokio runtime, so
            // await directly here rather than nesting another (which panics).
            let vector_store = arags_storage::VectorStore::open(&config.data_dir)
                .await
                .ok()
                .map(std::sync::Arc::new);
            let report =
                maintenance::run_maintenance(&project, &storage, vector_store, floor, dry_run)
                    .await
                    .context("maintenance failed")?;
            println!("Maintenance report (project={scope} dry_run={dry_run}):");
            println!(
                "  duplicate_chunks_removed         : {}",
                report.duplicate_chunks_removed
            );
            println!(
                "  low_confidence_patterns_removed  : {}",
                report.low_confidence_patterns_removed
            );
            println!(
                "  decayed_chunks                   : {}",
                report.decayed_chunks
            );
            println!("  kept                            : {}", report.kept);
        }
    }
    Ok(())
}
