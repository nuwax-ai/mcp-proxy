use crate::get_proxy_manager;
use crate::model::{CheckMcpStatusResponseStatus, GLOBAL_RESTART_TRACKER, McpType};
use crate::server::task::mcp_start_task::mcp_start_task;
use tokio::time::Duration;
use tracing::{error, info, warn};

// OneShot 服务超时时间：5分钟无活动则清理
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

// 连续健康检查失败阈值：连续失败 3 次才触发重启
const MAX_PROBE_FAILURES: u32 = 3;

/// 定期检查全局动态 router 里的 MCP 服务状态
///
/// ## 处理逻辑
///
/// 1. Error 状态 → 清理资源
/// 2. 空闲超时（5分钟）→ 清理资源（资源回收）
/// 3. 健康检查（只对 Ready 状态）
///    - Pending → 跳过（等待启动完成）
///    - Ready → 执行探测
///        - 成功 → 重置失败计数
///        - 失败 → 失败计数 + 1
///             - 连续失败 >= 3 → 重启后端服务
pub async fn schedule_check_mcp_live() {
    // 获取全局动态 router
    let proxy_manager = get_proxy_manager();
    // 获取所有 mcp 服务状态
    let mcp_service_statuses = proxy_manager.get_all_mcp_service_status();

    // 打印当前有多少个 mcp 插件服务在运行
    info!(
        "There are currently {} mcp plug-in services running",
        mcp_service_statuses.len()
    );

    // 遍历所有 mcp 服务状态
    for mcp_service_status in mcp_service_statuses {
        // 获取服务信息
        let mcp_id = mcp_service_status.mcp_id.clone();
        let mcp_type = mcp_service_status.mcp_type.clone();
        let cancellation_token = mcp_service_status.cancellation_token.clone();

        // 1. 如果 mcp 的状态是 ERROR，则清理资源
        if let CheckMcpStatusResponseStatus::Error(_) =
            mcp_service_status.check_mcp_status_response_status
        {
            if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                error!("Failed to cleanup resources for {}: {}", mcp_id, e);
            }
            continue;
        }

        // 根据 MCP 类型进行不同处理
        match mcp_type {
            McpType::Persistent => {
                // 检查持久化服务是否已被取消或子进程已终止
                if cancellation_token.is_cancelled() {
                    info!(
                        "The persistent MCP service {mcp_id} has been manually canceled and resources are being cleaned up."
                    );
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                    continue;
                }

                // 检查子进程是否还在运行
                if let Some(handler) = proxy_manager.get_proxy_handler(&mcp_id)
                    && handler.is_terminated_async().await
                {
                    info!(
                        "The persistent MCP service {mcp_id} child process ended abnormally and cleaned up resources."
                    );
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                }
            }
            McpType::OneShot => {
                // 2. 检查空闲超时（基于所有请求的活动）
                let idle_time = mcp_service_status.last_accessed.elapsed();

                // 空闲超时 → 清理资源
                if idle_time > ONESHOT_TIMEOUT {
                    info!(
                        "OneShot service {} idle timeout (idle time: {} seconds), clean up resources",
                        mcp_id,
                        idle_time.as_secs()
                    );
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                    continue;
                }

                // 3. 健康检查（只对 Ready 状态）
                // Pending 状态跳过探测，等待启动完成
                if !matches!(
                    mcp_service_status.check_mcp_status_response_status,
                    CheckMcpStatusResponseStatus::Ready
                ) {
                    // Pending 状态跳过探测
                    continue;
                }

                // 执行健康探测
                let handler = proxy_manager.get_proxy_handler(&mcp_id);
                if let Some(handler) = handler {
                    let is_terminated = handler.is_terminated_async().await;

                    if is_terminated {
                        let failures = proxy_manager.increment_probe_failures(&mcp_id);
                        info!(
                            "OneShot service {} health check failed ({}/{})",
                            mcp_id, failures, MAX_PROBE_FAILURES
                        );

                        if failures >= MAX_PROBE_FAILURES {
                            info!(
                                "OneShot service {} failed continuously {} times, triggering a restart",
                                mcp_id, failures
                            );
                            restart_mcp_service(&mcp_id, proxy_manager).await;
                        }
                    } else {
                        // 探测成功，重置失败计数
                        proxy_manager.reset_probe_failures(&mcp_id);
                    }
                }
            }
        }
    }
}

/// 重启 MCP 服务
///
/// ## 重启流程
///
/// 1. 检查重启冷却期（30秒）
/// 2. 获取配置（从服务状态或缓存）
/// 3. 清理旧资源（保留配置缓存）
/// 4. 重新启动服务（复用 mcp_start_task）
async fn restart_mcp_service(mcp_id: &str, proxy_manager: &crate::model::ProxyHandlerManager) {
    // 1. 检查重启冷却期
    if !GLOBAL_RESTART_TRACKER.can_restart(mcp_id) {
        info!(
            "Service {} is skipped during the restart cooling period.",
            mcp_id
        );
        return;
    }

    // 2. 获取配置（优先从服务状态，其次从缓存）
    let mcp_config = proxy_manager.get_mcp_config(mcp_id);
    let mcp_config = match mcp_config {
        Some(config) => Some(config),
        None => proxy_manager.get_mcp_config_from_cache(mcp_id).await,
    };

    let Some(mcp_config) = mcp_config else {
        warn!(
            "Service {} has no configuration and cannot be restarted. Clean up resources.",
            mcp_id
        );
        if let Err(e) = proxy_manager.cleanup_resources(mcp_id).await {
            error!("Failed to cleanup resources for {}: {}", mcp_id, e);
        }
        return;
    };

    // 3. 清理旧资源（保留配置缓存）
    if let Err(e) = proxy_manager.cleanup_resources_for_restart(mcp_id).await {
        error!("Cleanup service {} resource failed: {}", mcp_id, e);
        return;
    }

    // 4. 重新启动服务（复用 mcp_start_task，自动设置 Pending 状态）
    match mcp_start_task(mcp_config).await {
        Ok((_router, _cancellation_token)) => {
            // 重置失败计数（已在新的服务实例中初始化为 0）
            // 注意：此时 mcp_id 对应的是新的服务实例
            proxy_manager.reset_probe_failures(mcp_id);
            // 记录重启时间
            GLOBAL_RESTART_TRACKER.record_restart(mcp_id);
            info!("Service {} restarted successfully", mcp_id);
        }
        Err(e) => {
            error!("Service {} failed to restart: {}", mcp_id, e);
            // 重启失败，设置 Error 状态
            // 注意：此时服务已被清理，无法设置状态，只能记录日志
            // 下次请求到来时会触发重新启动
        }
    }
}
