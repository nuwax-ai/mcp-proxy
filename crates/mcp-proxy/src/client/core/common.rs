//! 公共模块 - 提取 SSE 和 Stream 模式的共享逻辑
//!
//! 减少代码重复，统一健康检查行为

use std::future::Future;
use std::time::Duration;

/// 健康检查能力 trait
///
/// 抽象 SSE 和 Stream handler 的共同行为
pub trait HealthChecker: Send + Sync {
    /// 检查后端是否可用（同步检查）
    fn is_backend_available(&self) -> bool;

    /// 检查是否已终止（异步，会调用 list_tools 验证后端服务）
    fn is_terminated_async(&self) -> impl Future<Output = bool> + Send;
}

/// 监控连接健康状态
///
/// 提取自 monitor_sse_connection 和 monitor_stream_connection 的公共逻辑
///
/// # 参数
/// - `handler`: 实现 HealthChecker trait 的 handler
/// - `ping_interval`: ping 间隔（秒），0 表示禁用 ping
/// - `ping_timeout`: ping 超时（秒）
/// - `quiet`: 是否静默模式
/// - `protocol_name`: 协议名称，用于日志输出（如 "SSE" 或 "Stream"）
///
/// # 返回值
/// 返回断开连接的原因描述
pub async fn monitor_connection_health<H: HealthChecker>(
    handler: &H,
    ping_interval: u64,
    ping_timeout: u64,
    quiet: bool,
    protocol_name: &str,
) -> String {
    let connection_start = std::time::Instant::now();
    if !quiet {
        tracing::info!("[{}] Connection health monitoring started", protocol_name);
    }

    // 健康检查日志间隔：与 ping 间隔一致，但至少 30 秒
    let health_log_interval_secs = if ping_interval > 0 {
        ping_interval.max(30)
    } else {
        30
    };

    if ping_interval == 0 {
        // 没有 ping 检测，仅检查连接状态
        let mut health_log_interval =
            tokio::time::interval(Duration::from_secs(health_log_interval_secs));
        health_log_interval.tick().await; // 跳过第一次
        let mut check_count = 0u64;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(1)) => {
                    if !handler.is_backend_available() {
                        let alive_duration = connection_start.elapsed();
                        let disconnect_reason =
                            format!(
                                "[{}] Backend connection closed (alive: {}s)",
                                protocol_name,
                                alive_duration.as_secs()
                            );
                        if !quiet {
                            tracing::error!("{}", disconnect_reason);
                        }
                        return disconnect_reason;
                    }
                }
                _ = health_log_interval.tick() => {
                    check_count += 1;
                    let alive_duration = connection_start.elapsed();
                    let backend_available = handler.is_backend_available();
                    tracing::info!(
                        "[{}] Health check #{} status={}, alive={}s",
                        protocol_name,
                        check_count,
                        if backend_available { "ok" } else { "error" },
                        alive_duration.as_secs()
                    );
                }
            }
        }
    }

    let mut interval = tokio::time::interval(Duration::from_secs(ping_interval));
    interval.tick().await;
    let mut ping_count = 0u64;
    let mut last_health_log = std::time::Instant::now();
    let mut first_ping = true; // 标记是否是第一次 ping

    loop {
        interval.tick().await;
        ping_count += 1;

        let alive_duration = connection_start.elapsed();

        if !handler.is_backend_available() {
            let disconnect_reason = format!(
                "[{}] Backend connection closed (alive: {}s)",
                protocol_name,
                alive_duration.as_secs()
            );
            if !quiet {
                tracing::error!("{}", disconnect_reason);
            }
            return disconnect_reason;
        }

        let check_result = tokio::time::timeout(
            Duration::from_secs(ping_timeout),
            handler.is_terminated_async(),
        )
        .await;

        match check_result {
            Ok(true) => {
                let disconnect_reason = format!(
                    "[{}] Ping check failed (service error), alive: {}s",
                    protocol_name,
                    alive_duration.as_secs()
                );
                if !quiet {
                    tracing::error!("{}", disconnect_reason);
                }
                return disconnect_reason;
            }
            Ok(false) => {
                // 第一次 ping 成功后打印，之后每隔 health_log_interval_secs 秒打印一次
                let since_last_log = last_health_log.elapsed().as_secs();
                if first_ping || since_last_log >= health_log_interval_secs {
                    tracing::info!(
                        "[{}] Ping check #{} passed, alive={}s",
                        protocol_name,
                        ping_count,
                        alive_duration.as_secs()
                    );
                    last_health_log = std::time::Instant::now();
                    first_ping = false;
                } else {
                    tracing::debug!(
                        "[{}] list_tools ping #{} passed, alive={}s",
                        protocol_name,
                        ping_count,
                        alive_duration.as_secs()
                    );
                }
            }
            Err(_) => {
                let disconnect_reason = format!(
                    "[{}] Ping check timeout ({}s), alive: {}s",
                    protocol_name,
                    ping_timeout,
                    alive_duration.as_secs()
                );
                if !quiet {
                    tracing::error!("{}", disconnect_reason);
                }
                return disconnect_reason;
            }
        }
    }
}
