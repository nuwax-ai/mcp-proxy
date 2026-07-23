mod cli;
mod config;
mod handlers;
mod models;
mod server;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use cli::{Cli, Commands, ModelsSubcommand};
use config::AppConfig;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // init tracing: server mode needs tower_http debug, CLI commands don't
    let default_filter = match &cli.command {
        Commands::Server(_) => "fastembed=info,tower_http=debug",
        _ => "fastembed=info",
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    match cli.command {
        Commands::Server(args) => {
            // 加载或生成配置
            let mut config = AppConfig::load_or_generate(args.config)?;

            // 命令行端口优先级最高：显式指定时覆盖配置文件 / env
            if let Some(port) = args.port {
                config.server.port = port;
            }

            // CLI model_url overrides config file / env
            if let Some(url) = args.model_url {
                config.fastembed.model_url = Some(url);
            }

            // CLI cache_dir overrides config file / env
            if let Some(dir) = args.cache_dir {
                config.fastembed.cache_dir = dir.to_string_lossy().to_string();
            }

            // Start server
            server::start_server(config).await?;
        }
        Commands::Models(models_cmd) => match models_cmd.command {
            ModelsSubcommand::Download(download_args) => {
                cli::models::download_model(download_args).await?;
            }
            ModelsSubcommand::List(list_args) => {
                cli::models::list_models(list_args).await?;
            }
            ModelsSubcommand::Pull(pull_args) => {
                cli::models::pull_model(pull_args).await?;
            }
        },
    }

    Ok(())
}
