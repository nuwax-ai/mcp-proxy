//! Stream 模式处理
//!
//! Streamable HTTP 协议的实现和连接管理

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

use super::common::HealthChecker;
use crate::client::support::{
    ConvertArgs, classify_error, print_diagnostic_report, summarize_error, truncate_str,
};
use crate::proxy::{McpClientConfig, StreamClientConnection, StreamProxyHandler, ToolFilter};

use mcp_streamable_proxy::{ServiceExt, stdio as stream_stdio};

/// 为 StreamProxyHandler 实现 HealthChecker trait
impl HealthChecker for StreamProxyHandler {
    fn is_backend_available(&self) -> bool {
        self.is_backend_available()
    }

    async fn is_terminated_async(&self) -> bool {
        self.is_terminated_async().await
    }
}

/// Stream 模式处理（使用 mcp-streamable-proxy，rmcp 0.12）
pub async fn run_stream_mode(
    config: McpClientConfig,
    args: ConvertArgs,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    tracing::info!("========================================");
    tracing::info!("Stream 模式启动");
    tracing::info!("目标 URL: {}", config.url);
    tracing::info!(
        "Ping 间隔: {}s, Ping 超时: {}s",
        args.ping_interval,
        args.ping_timeout
    );
    tracing::info!("========================================");

    if !quiet {
        eprintln!("🔗 正在连接到后端服务 (Stream)...");
    }

    // 1. 使用高层 API 连接
    let connect_timeout = Duration::from_secs(30);
    tracing::info!(
        "开始连接到后端服务 (超时: {}s)...",
        connect_timeout.as_secs()
    );
    let connect_start = std::time::Instant::now();

    let conn = tokio::time::timeout(
        connect_timeout,
        StreamClientConnection::connect(config.clone()),
    )
    .await
    .map_err(|_| {
        tracing::error!("连接后端超时 ({}s)", connect_timeout.as_secs());
        anyhow::anyhow!("连接后端超时 ({}秒)", connect_timeout.as_secs())
    })?
    .map_err(|e| {
        tracing::error!("连接后端失败: {}", e);
        anyhow::anyhow!("连接后端失败: {}", e)
    })?;

    let connect_duration = connect_start.elapsed();
    tracing::info!("后端连接成功 (耗时: {:?})", connect_duration);

    if !quiet {
        eprintln!("✅ 后端连接成功");
        // 打印工具列表
        print_stream_tools(&conn, quiet).await;
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

    // 3. 启动 stdio server（使用 stream_stdio，即 rmcp 0.12 的 stdio）
    tracing::info!("启动 stdio server...");
    let server = (*handler).clone().serve(stream_stdio()).await?;
    tracing::info!("stdio server 已启动");

    if !quiet {
        eprintln!("💡 stdio server 已启动，开始代理转换...");
    }

    // 4. 启动 watchdog 任务
    let handler_for_watchdog = handler.clone();
    let mut watchdog_handle = tokio::spawn(run_stream_watchdog(
        handler_for_watchdog,
        args,
        config,
        tool_filter,
        verbose,
        quiet,
    ));
    tracing::debug!("Watchdog 任务已启动");

    // 5. 等待 stdio server 退出
    tracing::info!("开始等待 stdio server 事件...");
    tokio::select! {
        result = server.waiting() => {
            tracing::info!("========================================");
            tracing::info!("stdio server 退出 - 原因: MCP 客户端断开连接 (stdin EOF)");
            tracing::info!("========================================");
            watchdog_handle.abort();
            result?;
        }
        watchdog_result = &mut watchdog_handle => {
            tracing::info!("========================================");
            tracing::info!("Watchdog 任务退出");
            tracing::info!("========================================");
            if let Err(e) = watchdog_result
                && !e.is_cancelled()
            {
                tracing::error!("Stream Watchdog task failed: {:?}", e);
            }
        }
    }

    tracing::info!("mcp-proxy convert (Stream 模式) 正常退出");
    Ok(())
}

/// 打印 Stream 连接的工具列表
async fn print_stream_tools(conn: &StreamClientConnection, quiet: bool) {
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

/// Stream 模式的 watchdog：负责监控连接健康、断开时重连
async fn run_stream_watchdog(
    handler: Arc<StreamProxyHandler>,
    args: ConvertArgs,
    config: McpClientConfig,
    _tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) {
    let max_retries = args.retries;
    let mut attempt = 0u32;
    let mut backoff_secs = 1u64;
    const MAX_BACKOFF_SECS: u64 = 30;
    let initial_connection_start = std::time::Instant::now();

    // 首先监控现有连接的健康状态
    let disconnect_reason =
        monitor_stream_connection(&handler, args.ping_interval, args.ping_timeout, quiet).await;

    // 连接断开，标记后端不可用
    handler.swap_backend(None);

    let alive_duration = initial_connection_start.elapsed();

    if !quiet {
        eprintln!("⚠️  连接断开: {}", disconnect_reason);
    }

    // 生成诊断报告（首次断开）
    print_diagnostic_report(
        "Streamable HTTP",
        &config.url,
        alive_duration.as_secs(),
        &disconnect_reason,
        None,
        args.logging.diagnostic,
    );

    // 进入重连循环
    loop {
        attempt += 1;

        if !quiet {
            eprintln!("🔗 正在重新连接 (第{}次尝试)...", attempt);
        }

        // 尝试建立连接
        let connect_result = StreamClientConnection::connect(config.clone()).await;

        match connect_result {
            Ok(conn) => {
                // 连接成功，获取 RunningService 并热替换后端
                let running = conn.into_running_service();
                handler.swap_backend(Some(running));
                backoff_secs = 1;

                if !quiet {
                    eprintln!("✅ 重连成功，恢复代理服务");
                }

                // 监控连接健康
                let reconnect_start = std::time::Instant::now();
                let disconnect_reason = monitor_stream_connection(
                    &handler,
                    args.ping_interval,
                    args.ping_timeout,
                    quiet,
                )
                .await;

                // 连接断开，标记后端不可用
                handler.swap_backend(None);
                let reconnect_alive_duration = reconnect_start.elapsed();

                if !quiet {
                    eprintln!("⚠️  连接断开: {}", disconnect_reason);
                }

                // 生成诊断报告（重连后断开）
                print_diagnostic_report(
                    "Streamable HTTP",
                    &config.url,
                    reconnect_alive_duration.as_secs(),
                    &disconnect_reason,
                    None,
                    args.logging.diagnostic,
                );
            }
            Err(e) => {
                let error_type = classify_error(&e);

                if max_retries > 0 && attempt >= max_retries {
                    if !quiet {
                        eprintln!("❌ 连接失败，已达最大重试次数 ({})", max_retries);
                        eprintln!("   错误类型: {}", error_type);
                        eprintln!("   错误详情: {}", e);
                    }
                    // 生成最终诊断报告
                    print_diagnostic_report(
                        "Streamable HTTP",
                        &config.url,
                        0,
                        "连接失败，达到最大重试次数",
                        Some(error_type),
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

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
    }
}

/// 监控 Stream 连接健康状态
///
/// 委托给 common::monitor_connection_health 公共函数
async fn monitor_stream_connection(
    handler: &StreamProxyHandler,
    ping_interval: u64,
    ping_timeout: u64,
    quiet: bool,
) -> String {
    super::common::monitor_connection_health(handler, ping_interval, ping_timeout, quiet, "Stream")
        .await
}
