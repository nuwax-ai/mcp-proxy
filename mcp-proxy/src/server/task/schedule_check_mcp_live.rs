use crate::get_proxy_manager;
use crate::model::{CheckMcpStatusResponseStatus, McpType};
use tokio::time::Duration;
use tracing::{error, info};

// OneShot 服务超时时间：5分钟无活动则清理
const ONESHOT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

//定期检查 全局动态router里的 短时 mcp服务,如果超过5分钟,没有被访问,则认为服务已经结束,则清理资源
pub async fn schedule_check_mcp_live() {
    //获取全局动态router
    let proxy_manager = get_proxy_manager();
    //获取所有mcp服务状态
    let mcp_service_statuses = proxy_manager.get_all_mcp_service_status();

    //打印当前有多少个mcp插件服务,在运行
    info!("当前有 {} 个mcp插件服务在运行", mcp_service_statuses.len());

    //遍历所有mcp服务状态
    for mcp_service_status in mcp_service_statuses {
        //获取服务信息
        let mcp_id = mcp_service_status.mcp_id.clone();
        let mcp_type = mcp_service_status.mcp_type.clone();
        let cancellation_token = mcp_service_status.cancellation_token.clone();

        //如果 mcp的状态是 ERROR,则清理资源
        if let CheckMcpStatusResponseStatus::Error(_) =
            mcp_service_status.check_mcp_status_response_status
        {
            if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                error!("Failed to cleanup resources for {}: {}", mcp_id, e);
            }
            continue;
        }

        //根据MCP类型进行不同处理
        match mcp_type {
            McpType::Persistent => {
                //检查持久化服务是否已被取消或子进程已终止
                if cancellation_token.is_cancelled() {
                    info!("持久化 MCP 服务 {mcp_id} 已被手动取消，清理资源");
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                    continue;
                }

                //检查子进程是否还在运行
                if let Some(handler) = proxy_manager.get_proxy_handler(&mcp_id)
                    && handler.is_terminated_async().await
                {
                    info!("持久化 MCP 服务 {mcp_id} 子进程异常结束，清理资源");
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                }
            }
            McpType::OneShot => {
                // 检查后端进程是否已结束
                let handler_terminated =
                    if let Some(handler) = proxy_manager.get_proxy_handler(&mcp_id) {
                        handler.is_terminated_async().await
                    } else {
                        // handler 不存在，说明已被清理
                        true
                    };

                // 检查最后访问时间（基于所有请求的活动）
                let idle_time = mcp_service_status.last_accessed.elapsed();

                // OneShot 服务清理条件：后端结束 或 超过5分钟无活动
                if handler_terminated || idle_time > ONESHOT_TIMEOUT {
                    info!(
                        "OneShot 服务 {} 清理资源（后端结束: {}, 空闲时间: {}秒）",
                        mcp_id,
                        handler_terminated,
                        idle_time.as_secs()
                    );
                    if let Err(e) = proxy_manager.cleanup_resources(&mcp_id).await {
                        error!("Failed to cleanup resources for {}: {}", mcp_id, e);
                    }
                }
            }
        }
    }
}
