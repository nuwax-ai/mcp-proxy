use crate::get_proxy_manager;
use crate::model::{CheckMcpStatusResponseStatus, McpType};
use tokio::time::Duration;
use tracing::{debug, error, info};

// OneShot 服务超时时间：5分钟无活动则清理
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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
                if let Some(handler) = handler
                    && handler.is_terminated_async().await
                {
                    // OneShot 进程退出是**正常终态**（脚本/任务执行完即退出）。
                    // 与 router 层约定一致（mcp_dynamic_router_service：
                    // "OneShot 只清理、不重启"），此处不再计数自动重启——旧逻辑
                    // 会把已完成的 OneShot 每 ~60s 无限拉起再退出。保留实例等
                    // 空闲超时（ONESHOT_TIMEOUT）统一清理，给客户端留出
                    // check_status 取终态的窗口（0006/0007 语义）。
                    debug!(
                        "OneShot service {} process exited (normal end state); \
                         kept until idle-timeout cleanup",
                        mcp_id
                    );
                }
            }
        }
    }
}
