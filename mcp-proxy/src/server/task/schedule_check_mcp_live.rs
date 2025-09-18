use crate::get_proxy_manager;
use crate::model::{CheckMcpStatusResponseStatus, McpType};
use log::info;
use tokio::time::Duration;

//定期检查 全局动态router里的 短时 mcp服务,如果超过3分钟,没有被访问,则认为服务已经结束,则清理资源
pub async fn schedule_check_mcp_live() {
    //获取全局动态router
    let proxy_manager = get_proxy_manager();
    //获取所有mcp服务状态
    let mcp_service_statuses = proxy_manager.get_all_mcp_service_status();

    //打印当前有多少个mcp插件服务,在运行
    info!("当前有 {} 个mcp插件服务在运行", mcp_service_statuses.len());
    //定义超时时间（3分钟）
    let timeout_duration = Duration::from_secs(5 * 60);

    //遍历所有mcp服务状态
    for mcp_service_status in mcp_service_statuses {
        //获取服务信息
        let mcp_id = mcp_service_status.mcp_id.clone();
        let mcp_type = mcp_service_status.mcp_type.clone();
        let cancellation_token = mcp_service_status.cancellation_token.clone();
        let _mcp_protocol = mcp_service_status.mcp_router_path.mcp_protocol;

        //如果 mcp的状态是 ERROR,则清理资源
        if let CheckMcpStatusResponseStatus::Error(_) =
            mcp_service_status.check_mcp_status_response_status
        {
            proxy_manager.cleanup_resources(&mcp_id).await;
            continue;
        }

        //根据MCP类型进行不同处理
        match mcp_type {
            McpType::Persistent => {
                //检查持久化服务是否已被取消或子进程已终止
                if cancellation_token.is_cancelled() {
                    info!("持久化 MCP 服务 {mcp_id} 已被手动取消，清理资源");
                    proxy_manager.cleanup_resources(&mcp_id).await;
                    continue;
                }

                //检查子进程是否还在运行
                if let Some(handler) = proxy_manager.get_proxy_handler(&mcp_id) {
                    if handler.is_terminated_async().await {
                        info!("持久化 MCP 服务 {mcp_id} 子进程异常结束，清理资源");
                        proxy_manager.cleanup_resources(&mcp_id).await;
                    }
                }
            }
            McpType::OneShot => {
                //检查一次性任务是否已被取消
                if cancellation_token.is_cancelled() {
                    info!("一次性 MCP 任务 {mcp_id} 已被手动取消，清理资源");
                    proxy_manager.cleanup_resources(&mcp_id).await;
                    continue;
                }

                //检查子进程是否已经完成
                if let Some(handler) = proxy_manager.get_proxy_handler(&mcp_id) {
                    if handler.is_terminated_async().await {
                        info!("一次性 MCP 任务 {mcp_id} 已完成，开始清理资源");
                        proxy_manager.cleanup_resources(&mcp_id).await;
                        info!("一次性 MCP 任务 {mcp_id} 资源清理完成");
                        continue;
                    }

                    //检查是否超过3分钟未访问
                    let idle_time = mcp_service_status.last_accessed.elapsed();
                    if idle_time > timeout_duration {
                        info!("一次性 MCP 任务 {mcp_id} 超过3分钟未被访问，自动清理资源");
                        proxy_manager.cleanup_resources(&mcp_id).await;
                        info!("一次性 MCP 任务 {mcp_id} 资源自动清理完成");
                    }
                } else {
                    //处理器已经被移除
                    info!("一次性 MCP 任务 {mcp_id} 不存在，无需清理");
                }
            }
        }
    }
}
