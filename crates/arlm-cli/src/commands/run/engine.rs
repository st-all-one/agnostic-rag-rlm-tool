use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info, instrument, warn};

use crate::commands::run::config::RunConfig;
use crate::commands::run::{finalize, live, setup};
use crate::output;

#[allow(
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::cast_possible_truncation
)]
#[instrument(skip(config), fields(task = %config.task, live = config.live))]
pub async fn execute(config: RunConfig<'_>) -> Result<()> {
    let _timer = arlm_core::logging::ScopedTimer::new("cli_run");
    let start = Instant::now();
    let backend_name = config.backend.unwrap_or("ollama").to_string();

    if !config.llm {
        anyhow::bail!(
            "`arlm run` requires --llm flag. Use `arlm search` or `arlm context` for deterministic operations."
        );
    }

    let project_name = setup::project_name(config.project);
    info!(project = %project_name, "resolved project");

    let kind = setup::parse_backend(config.backend)?;
    let api_key = setup::load_api_key(kind);
    let llm_backend =
        arlm_llm::get_backend(&kind, api_key, None).context("failed to create LLM backend")?;
    debug!(backend = %backend_name, "created llm backend");

    let run_id = format!("run-{}", uuid::Uuid::now_v7().as_simple());
    info!(run_id = %run_id, "starting run");

    let effective_task = setup::resolve_effective_task(&config);

    if config.verbose {
        output::info(&format!(
            "Starting RLM run {run_id} (backend={backend_name}, depth={}, max_nodes={})",
            config.depth, config.max_nodes
        ));
    }

    let progress = if config.live {
        None
    } else {
        let p = indicatif::ProgressBar::new_spinner();
        p.set_style(
            indicatif::ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg} [{elapsed_precise}]")
                .map_err(|e| anyhow::anyhow!("invalid template: {e}"))?,
        );
        p.set_message(format!("Running RLM: {}", config.task));
        Some(p)
    };

    let abort = arlm_core::AbortSignal::new();
    let abort_for_signal = abort.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            output::warn("Cancelling run...");
            warn!("received ctrl-c, cancelling");
            abort_for_signal.cancel();
        }
    });

    let input =
        setup::build_run_input(&config, kind, project_name, &run_id, abort, &effective_task);

    let result = if config.live {
        live::run_live(&config, input, llm_backend).await?
    } else {
        arlm_core::run_rlm_engine(input, llm_backend)
            .await
            .context("RLM engine failed")?
    };

    if let Some(p) = progress {
        p.finish_and_clear();
    }

    finalize::persist_run(&result, &config)?;
    finalize::save_session(&result, &config);
    let rendered = finalize::print_output(&result, &config);
    print!("{rendered}");

    if config.persist {
        if let Err(e) = crate::commands::persist::save_page(
            config.task,
            &rendered,
            config.project,
            config.format,
        ) {
            warn!(error = %e, "failed to persist run output");
        }
    }

    debug!(
        elapsed_ms = start.elapsed().as_millis() as u64,
        "cli run complete"
    );
    Ok(())
}
