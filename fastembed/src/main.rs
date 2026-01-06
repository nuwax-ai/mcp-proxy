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
    // 初始化日志
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fastembed=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Server(args) => {
            // 加载或生成配置
            let mut config = AppConfig::load_or_generate(args.config)?;

            // 命令行端口覆盖配置文件
            if args.port != 8080 {
                config.server.port = args.port;
            }

            // 启动服务器
            server::start_server(config).await?;
        }
        Commands::Models(models_cmd) => match models_cmd.command {
            ModelsSubcommand::Download(download_args) => {
                cli::models::download_model(download_args).await?;
            }
            ModelsSubcommand::List(list_args) => {
                cli::models::list_models(list_args).await?;
            }
        },
    }

    Ok(())
}
