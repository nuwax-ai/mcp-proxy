//! SSE 模式处理
//!
//! Server-Sent Events 协议的实现和连接管理

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tracing::error;

use super::common::HealthChecker;
use crate::client::support::{
    ConvertArgs, classify_error, print_diagnostic_report, summarize_error, truncate_str,
};
use crate::proxy::{McpClientConfig, ProxyHandler, SseClientConnection, ToolFilter};
use crate::t;

use mcp_sse_proxy::{ServiceExt, stdio as sse_stdio};

/// 为 ProxyHandler 实现 HealthChecker trait
impl HealthChecker for ProxyHandler {
    fn is_backend_available(&self) -> bool {
        self.is_backend_available()
    }

    async fn is_terminated_async(&self) -> bool {
        self.is_terminated_async().await
    }
}

/// SSE 模式处理（使用 mcp-sse-proxy，rmcp 0.10）
pub async fn run_sse_mode(
    config: McpClientConfig,
    args: ConvertArgs,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    tracing::info!("========================================");
    tracing::info!("{}", t!("cli.sse.mode_starting"));
    tracing::info!("{}", t!("cli.convert.target_url", url = config.url));
    tracing::info!(
        "{}",
        t!("cli.convert.ping_config",
            interval = args.ping_interval,
            timeout = args.ping_timeout
        )
    );
    tracing::info!("========================================");

    if !quiet {
        eprintln!("🔗 正在连接到后端服务 (SSE)...");
    }

    // 1. 使用高层 API 连接
    let connect_timeout = Duration::from_secs(30);
    tracing::info!(
        "{}",
        t!("cli.convert.connecting_backend", timeout = connect_timeout.as_secs())
    );
    let connect_start = std::time::Instant::now();

    let conn = tokio::time::timeout(
        connect_timeout,
        SseClientConnection::connect(config.clone()),
    )
    .await
    .map_err(|_| {
        tracing::error!("{}", t!("cli.sse.connect_timeout", seconds = connect_timeout.as_secs()));
        anyhow::anyhow!("Backend connection timeout ({}s)", connect_timeout.as_secs())
    })?
    .map_err(|e| {
        tracing::error!("{}", t!("cli.sse.connect_failed", error = e.to_string()));
        anyhow::anyhow!("Backend connection failed: {}", e)
    })?;

    let connect_duration = connect_start.elapsed();
    tracing::info!("{}", t!("cli.sse.connect_success", duration = format!("{:?}", connect_duration)));

    if !quiet {
        eprintln!("✅ 后端连接成功");
        // 打印工具列表
        print_sse_tools(&conn, quiet).await;
        if args.ping_interval > 0 {
            eprintln!(
                "💓 心跳检测: 每 {}s ping 一次（超时 {}s）",
                args.ping_interval, args.ping_timeout
            );
        }
    }

    // 2. 创建 handler（消耗 conn）
    tracing::debug!("创建 ProxyHandler...");
    let handler = Arc::new(conn.into_handler("cli".to_string(), tool_filter.clone()));
    tracing::debug!("ProxyHandler 创建完成");

    // 3. 启动 stdio server
    tracing::info!("{}", t!("cli.sse.stdio_starting"));
    let server = (*handler).clone().serve(sse_stdio()).await?;
    tracing::info!("{}", t!("cli.sse.stdio_started"));

    if !quiet {
        eprintln!("💡 stdio server 已启动，开始代理转换...");
    }

    // 4. 启动 watchdog 任务
    let handler_for_watchdog = handler.clone();
    let mut watchdog_handle = tokio::spawn(run_sse_watchdog(
        handler_for_watchdog,
        args,
        config,
        tool_filter,
        verbose,
        quiet,
    ));
    tracing::debug!("Watchdog 任务已启动");

    // 5. 等待 stdio server 退出
    tracing::info!("{}", t!("cli.sse.waiting_events"));
    tokio::select! {
        result = server.waiting() => {
            tracing::info!("========================================");
            tracing::info!("{}", t!("cli.sse.stdio_exit_eof"));
            tracing::info!("========================================");
            watchdog_handle.abort();
            result?;
        }
        watchdog_result = &mut watchdog_handle => {
            tracing::info!("========================================");
            tracing::info!("{}", t!("cli.sse.watchdog_exit"));
            tracing::info!("========================================");
            if let Err(e) = watchdog_result
                && !e.is_cancelled()
            {
                error!("SSE Watchdog task failed: {:?}", e);
            }
        }
    }

    tracing::info!("{}", t!("cli.sse.normal_exit"));
    Ok(())
}

/// 打印 SSE 连接的工具列表
async fn print_sse_tools(conn: &SseClientConnection, quiet: bool) {
    if quiet {
        return;
    }
    match conn.list_tools().await {
        Ok(tools) => {
            if tools.is_empty() {
                eprintln!("⚠️  工具列表为空 (tools/list 返回 0 个工具)");
            } else {
                eprintln!("🔧 可用工具 ({} 个):", tools.len());
                for tool in &tools {
                    let desc = tool.description.as_deref().unwrap_or("无描述");
                    let desc_short = truncate_str(desc, 50);
                    eprintln!("   - {} : {}", tool.name, desc_short);
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  获取工具列表失败: {}", e);
        }
    }
}

/// SSE 模式的 watchdog：负责监控连接健康、断开时重连
async fn run_sse_watchdog(
    handler: Arc<ProxyHandler>,
    args: ConvertArgs,
    config: McpClientConfig,
    _tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) {
    tracing::info!("========================================");
    tracing::info!("{}", t!("cli.sse.watchdog_starting"));
    tracing::info!("{}", t!("cli.sse.max_retries", count = args.retries));
    tracing::info!("========================================");

    let max_retries = args.retries;
    let mut attempt = 0u32;
    let mut backoff_secs = 1u64;
    const MAX_BACKOFF_SECS: u64 = 30;
    let initial_connection_start = std::time::Instant::now();

    // 首先监控现有连接的健康状态
    tracing::info!("{}", t!("cli.sse.monitoring_connection"));
    let disconnect_reason =
        monitor_sse_connection(&handler, args.ping_interval, args.ping_timeout, quiet).await;

    // 连接断开，标记后端不可用
    tracing::warn!("{}", t!("cli.sse.initial_disconnect", reason = disconnect_reason));
    handler.swap_backend(None);

    let alive_duration = initial_connection_start.elapsed();
    tracing::info!("{}", t!("cli.sse.connection_alive", seconds = alive_duration.as_secs()));

    if !quiet {
        eprintln!("⚠️  连接断开: {}", disconnect_reason);
    }

    // 生成诊断报告（首次断开）
    print_diagnostic_report(
        "SSE",
        &config.url,
        alive_duration.as_secs(),
        &disconnect_reason,
        None,
        args.logging.diagnostic,
    );

    // 进入重连循环
    loop {
        attempt += 1;
        tracing::info!("========================================");
        tracing::info!("{}", t!("cli.sse.reconnect_attempt", attempt = attempt, max = max_retries));
        tracing::info!("{}", t!("cli.sse.backoff_time", seconds = backoff_secs));

        if !quiet {
            eprintln!("🔗 正在重新连接 (第{}次尝试)...", attempt);
        }

        // 尝试建立连接
        tracing::debug!("开始建立连接...");
        let connect_start = std::time::Instant::now();
        let connect_result = SseClientConnection::connect(config.clone()).await;
        let connect_duration = connect_start.elapsed();

        match connect_result {
            Ok(conn) => {
                tracing::info!("{}", t!("cli.sse.reconnect_success", duration = format!("{:?}", connect_duration)));

                // 连接成功，获取 RunningService 并热替换后端
                let running = conn.into_running_service();
                handler.swap_backend(Some(running));
                backoff_secs = 1;

                if !quiet {
                    eprintln!("✅ 重连成功，恢复代理服务");
                }

                // 监控连接健康
                tracing::info!("{}", t!("cli.sse.monitoring_reconnect"));
                let reconnect_start = std::time::Instant::now();
                let disconnect_reason =
                    monitor_sse_connection(&handler, args.ping_interval, args.ping_timeout, quiet)
                        .await;

                // 连接断开，标记后端不可用
                tracing::warn!("{}", t!("cli.sse.reconnect_disconnect", reason = disconnect_reason));
                handler.swap_backend(None);
                let reconnect_alive_duration = reconnect_start.elapsed();
                tracing::info!("{}", t!("cli.sse.reconnect_alive", seconds = reconnect_alive_duration.as_secs()));

                if !quiet {
                    eprintln!("⚠️  连接断开: {}", disconnect_reason);
                }

                // 生成诊断报告（重连后断开）
                print_diagnostic_report(
                    "SSE",
                    &config.url,
                    reconnect_alive_duration.as_secs(),
                    &disconnect_reason,
                    None,
                    args.logging.diagnostic,
                );
            }
            Err(e) => {
                let error_type = classify_error(&e);
                tracing::error!(
                    "连接失败 [{}]: {} (耗时: {:?})",
                    error_type,
                    summarize_error(&e),
                    connect_duration
                );

                if max_retries > 0 && attempt >= max_retries {
                    tracing::error!("{}", t!("cli.sse.max_retries_reached", count = max_retries));
                    if !quiet {
                        eprintln!("❌ 连接失败，已达最大重试次数 ({})", max_retries);
                        eprintln!("   错误类型: {}", error_type);
                        eprintln!("   错误详情: {}", e);
                    }
                    // 生成最终诊断报告
                    print_diagnostic_report(
                        "SSE",
                        &config.url,
                        0,
                        "连接失败，达到最大重试次数",
                        Some(&error_type),
                        args.logging.diagnostic,
                    );
                    break;
                }

                if !quiet {
                    if max_retries == 0 {
                        eprintln!(
                            "⚠️  连接失败 [{}]: {}，{}秒后重连 (第{}次)...",
                            error_type,
                            summarize_error(&e),
                            backoff_secs,
                            attempt
                        );
                    } else {
                        eprintln!(
                            "⚠️  连接失败 [{}]: {}，{}秒后重连 ({}/{})...",
                            error_type,
                            summarize_error(&e),
                            backoff_secs,
                            attempt,
                            max_retries
                        );
                    }
                }

                if verbose && !quiet {
                    eprintln!("   完整错误: {}", e);
                }
            }
        }

        tracing::debug!("等待 {}s 后下次重连...", backoff_secs);
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }

    tracing::info!("{}", t!("cli.sse.watchdog_exit_msg"));
}

/// 监控 SSE 连接健康状态
///
/// 委托给 common::monitor_connection_health 公共函数
async fn monitor_sse_connection(
    handler: &ProxyHandler,
    ping_interval: u64,
    ping_timeout: u64,
    quiet: bool,
) -> String {
    super::common::monitor_connection_health(handler, ping_interval, ping_timeout, quiet, "SSE")
        .await
}
