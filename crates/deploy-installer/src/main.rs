use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = deploy_installer::cli::Cli::parse();
    deploy_installer::cli::run(cli)
}
