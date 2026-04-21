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
    tracing::info!("Starting Stream mode");
    tracing::info!("Target URL: {}", config.url);
    tracing::info!(
        "Ping config: interval={}s, timeout={}s",
        args.ping_interval,
        args.ping_timeout
    );
    tracing::info!("========================================");

    if !quiet {
        eprintln!("🔗 Connecting to backend service (Stream)...");
    }

    // 1. 使用高层 API 连接（带重试，防止初始连接因时序问题失败）
    //
    // 重试策略：最多 3 次，每次给 10s 连接超时，间隔 2s/4s
    // 第1次（最多 10s）→ 失败 → 等 2s → 第2次（最多 10s）→ 失败 → 等 4s → 第3次（最多 10s）→ 退出
    // 最坏总耗时：10 + 2 + 10 + 4 + 10 = 36s（但大多数失败会比 10s 更快返回）
    // 通常总耗时：连接快速失败(~1s) + 2 + 快速失败(~1s) + 4 + 快速失败(~1s) = ~9s
    let connect_timeout = Duration::from_secs(10);
    const MAX_INITIAL_RETRIES: u32 = 3;
    const INITIAL_BACKOFF_SECS: u64 = 2;
    const MAX_BACKOFF_SECS: u64 = 4;

    tracing::info!(
        "Connecting to backend (per-attempt timeout: {}s, max retries: {})",
        connect_timeout.as_secs(),
        MAX_INITIAL_RETRIES
    );
    let connect_start = std::time::Instant::now();

    let mut last_error = None;
    let mut conn = None;
    let mut backoff_secs = INITIAL_BACKOFF_SECS;
    for attempt in 1..=MAX_INITIAL_RETRIES {
        match tokio::time::timeout(
            connect_timeout,
            StreamClientConnection::connect(config.clone()),
        )
        .await
        {
            Ok(Ok(c)) => {
                if attempt > 1 {
                    tracing::info!(
                        "Backend connection succeeded on attempt {}/{}",
                        attempt,
                        MAX_INITIAL_RETRIES
                    );
                }
                conn = Some(c);
                break;
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    "Backend connection attempt {}/{} failed: {}",
                    attempt,
                    MAX_INITIAL_RETRIES,
                    e
                );
                last_error = Some(format!("Backend connection failed: {}", e));
            }
            Err(_) => {
                tracing::warn!(
                    "Backend connection attempt {}/{} timed out ({}s)",
                    attempt,
                    MAX_INITIAL_RETRIES,
                    connect_timeout.as_secs()
                );
                last_error = Some(format!(
                    "Backend connection timeout ({}s)",
                    connect_timeout.as_secs()
                ));
            }
        }
        if attempt < MAX_INITIAL_RETRIES {
            tracing::info!("Retrying in {}s... (elapsed: {:?})", backoff_secs, connect_start.elapsed());
            if !quiet {
                eprintln!(
                    "⚠️ Connection attempt {}/{} failed, retrying in {}s...",
                    attempt, MAX_INITIAL_RETRIES, backoff_secs
                );
            }
            tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
            backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
        }
    }

    let conn = conn.ok_or_else(|| {
        let elapsed = connect_start.elapsed();
        let msg = last_error.unwrap_or_else(|| "Unknown connection error".to_string());
        tracing::error!(
            "All {} connection attempts failed after {:?}: {}",
            MAX_INITIAL_RETRIES, elapsed, msg
        );
        eprintln!(
            "❌ All {} connection attempts failed after {:.1}s: {}",
            MAX_INITIAL_RETRIES, elapsed.as_secs_f64(), msg
        );
        anyhow::anyhow!(msg)
    })?;

    let connect_duration = connect_start.elapsed();
    tracing::info!(
        "Backend connected successfully (duration: {:?})",
        connect_duration
    );

    if !quiet {
        eprintln!("✅ Backend connected successfully");
        // 打印工具列表
        print_stream_tools(&conn, quiet).await;
        if args.ping_interval > 0 {
            eprintln!(
                "💓 Health ping: every {}s (timeout {}s)",
                args.ping_interval, args.ping_timeout
            );
        }
    }

    // 2. 创建 handler（消耗 conn）
    tracing::debug!("Creating ProxyHandler...");
    let handler = Arc::new(conn.into_handler("cli".to_string(), tool_filter.clone()));
    tracing::debug!("ProxyHandler created");

    // 3. 启动 stdio server（使用 stream_stdio，即 rmcp 0.12 的 stdio）
    tracing::info!("Starting stdio server...");
    let server = (*handler).clone().serve(stream_stdio()).await.map_err(|e| {
        tracing::error!("Failed to start stdio server: {:?}", e);
        eprintln!("❌ Failed to start stdio server: {}", e);
        e
    })?;
    tracing::info!("Stdio server started");

    if !quiet {
        eprintln!("💡 Stdio server started, proxying traffic...");
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
    tracing::debug!("Watchdog task started");

    // 5. 等待 stdio server 退出
    tracing::info!("Waiting for stdio/watchdog events...");
    tokio::select! {
        result = server.waiting() => {
            tracing::info!("========================================");
            tracing::info!("Stdio server exited (EOF)");
            tracing::info!("========================================");
            watchdog_handle.abort();
            result?;
        }
        watchdog_result = &mut watchdog_handle => {
            tracing::info!("========================================");
            tracing::info!("Watchdog task exited");
            tracing::info!("========================================");
            if let Err(e) = watchdog_result
                && !e.is_cancelled()
            {
                tracing::error!("Stream Watchdog task failed: {:?}", e);
            }
        }
    }

    tracing::info!("Stream mode exited normally");
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
                eprintln!("⚠️  Tool list is empty (tools/list returned 0 tools)");
            } else {
                eprintln!("🔧 Available tools ({}):", tools.len());
                for tool in &tools {
                    let desc = tool.description.as_deref().unwrap_or("no description");
                    let desc_short = truncate_str(desc, 50);
                    eprintln!("   - {} : {}", tool.name, desc_short);
                }
            }
        }
        Err(e) => {
            eprintln!("⚠️  Failed to list tools: {}", e);
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
    const EVENT_DISCONNECTED: &str = "EVENT_DISCONNECTED";
    const EVENT_RECONNECTED: &str = "EVENT_RECONNECTED";
    const EVENT_RETRY_BACKOFF: &str = "EVENT_RETRY_BACKOFF";
    let initial_connection_start = std::time::Instant::now();

    // 首先监控现有连接的健康状态
    let disconnect_reason =
        monitor_stream_connection(&handler, args.ping_interval, args.ping_timeout, quiet).await;

    // 连接断开，标记后端不可用
    handler.swap_backend(None);

    let alive_duration = initial_connection_start.elapsed();

    if !quiet {
        eprintln!(
            "⚠️ [{}] Connection disconnected: {}",
            EVENT_DISCONNECTED, disconnect_reason
        );
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
            eprintln!("🔗 Reconnecting (attempt #{})...", attempt);
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
                    eprintln!(
                        "✅ [{}] Reconnected, proxy service resumed",
                        EVENT_RECONNECTED
                    );
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
                    eprintln!(
                        "⚠️ [{}] Connection disconnected: {}",
                        EVENT_DISCONNECTED, disconnect_reason
                    );
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
                        eprintln!(
                            "❌ Connection failed, max retries reached ({})",
                            max_retries
                        );
                        eprintln!("   Error type: {}", error_type);
                        eprintln!("   Error detail: {}", e);
                    }
                    // 生成最终诊断报告
                    print_diagnostic_report(
                        "Streamable HTTP",
                        &config.url,
                        0,
                        "Connection failed: max retries reached",
                        Some(&error_type),
                        args.logging.diagnostic,
                    );
                    break;
                }

                if !quiet {
                    if max_retries == 0 {
                        eprintln!(
                            "⚠️ [{}] Connection failed [{}]: {}; retrying in {}s (attempt #{})...",
                            EVENT_RETRY_BACKOFF,
                            error_type,
                            summarize_error(&e),
                            backoff_secs,
                            attempt
                        );
                    } else {
                        eprintln!(
                            "⚠️ [{}] Connection failed [{}]: {}; retrying in {}s ({}/{})...",
                            EVENT_RETRY_BACKOFF,
                            error_type,
                            summarize_error(&e),
                            backoff_secs,
                            attempt,
                            max_retries
                        );
                    }
                }

                if verbose && !quiet {
                    eprintln!("   Full error: {}", e);
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
