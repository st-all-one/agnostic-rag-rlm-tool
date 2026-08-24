use anyhow::Result;
use arlm_cli::cli::Cli;
use arlm_cli::dispatch;
use clap::Parser;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<()> {
    let cli = Cli::parse();

    arlm_core::logging::init_logging(cli.verbose);

    let rt = tokio::runtime::Runtime::new()?;

    dispatch::dispatch(cli, &rt)
}
