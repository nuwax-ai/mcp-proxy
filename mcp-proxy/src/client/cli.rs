// MCP-Proxy CLI 实现
//
// CLI 主入口，负责命令分发到各个实现模块

use anyhow::{Result, bail};

// 导出 CLI 特定类型
// 注意: ConvertArgs 等通用参数类型已由 client/mod.rs 统一导出
pub use crate::client::support::args::{Cli, Commands};

/// 运行 CLI 主逻辑
///
/// 根据命令类型分发到对应的实现模块：
/// - Convert -> cli_impl/convert_cmd.rs
/// - Check/Detect -> cli_impl/check.rs
/// - Proxy -> proxy_server.rs
pub async fn run_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Convert(args)) => {
            crate::client::cli_impl::run_convert_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Check(args)) => {
            crate::client::cli_impl::run_check_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Detect(args)) => {
            crate::client::cli_impl::run_detect_command(args, cli.verbose, cli.quiet).await
        }
        Some(Commands::Proxy(args)) => {
            super::proxy_server::run_proxy_command(args, cli.verbose, cli.quiet).await
        }
        None => {
            // 直接 URL 模式（向后兼容）
            if let Some(url) = cli.url {
                let args = crate::client::support::args::ConvertArgs {
                    url: Some(url),
                    config: None,
                    config_file: None,
                    name: None,
                    protocol: None,
                    auth: None,
                    header: vec![],
                    retries: 0,    // 无限重试
                    allow_tools: None,
                    deny_tools: None,
                    ping_interval: 30,  // 默认 30 秒 ping 一次
                    ping_timeout: 10,   // 默认 10 秒超时
                    logging: crate::client::support::LoggingArgs {
                        diagnostic: true,   // 默认启用诊断模式
                        log_dir: None,      // 默认无日志目录（将在 init_logging 中自动设置）
                        log_file: None,     // 默认无日志文件
                        otlp_endpoint: None, // 默认不启用 OTLP 追踪
                        service_name: "mcp-proxy".to_string(),
                    },
                };
                crate::client::cli_impl::run_convert_command(args, cli.verbose, cli.quiet).await
            } else {
                bail!("请提供 URL 或使用子命令")
            }
        }
    }
}
