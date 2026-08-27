#![cfg_attr(
    test,
    allow(
        unsafe_code,
        clippy::expect_used,
        clippy::unwrap_used,
        clippy::panic,
        clippy::needless_borrow,
        clippy::unnecessary_literal_bound,
        clippy::float_cmp,
        clippy::duration_suboptimal_units,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )
)]
// Pedantic stylistic lints that are pervasive across this CLI command surface.
#![allow(
    clippy::missing_errors_doc,
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::match_same_arms
)]

pub mod auth_client;
pub mod backend;
pub mod cli;
pub mod client;
pub mod commands;
pub mod dispatch;
pub mod gitignore;
pub mod llm_post;
pub mod output;
pub mod prompts;
pub mod user_config;
pub mod volunteer;
pub mod watcher;

pub use client::{ClientConfig, create_client};
pub use output::{Format, error, info, success, warn};
