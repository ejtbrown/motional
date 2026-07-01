use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use motional_clients::service_runtime::{run_service, ServiceRunOptions};

#[derive(Debug, Parser)]
#[command(author, version, about = "Background service for Motional automation")]
struct Cli {
    #[arg(long, env = "MOTIONAL_CONFIG")]
    config: Option<PathBuf>,

    #[arg(long, env = "MOTIONAL_DRY_RUN")]
    dry_run: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut options = ServiceRunOptions::default();
    if let Some(path) = cli.config {
        options.config_path = path;
    }
    options.dry_run = cli.dry_run;

    run_service(options)
}
