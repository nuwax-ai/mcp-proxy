use crate::server::task::schedule_check_mcp_live;
use log::{debug, info, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, interval};

/// 启动定时任务，定期检查MCP服务状态
///
/// 这个函数会创建一个tokio定时任务，每隔指定的时间间隔执行一次`schedule_check_mcp_live`函数
/// 用于检查和清理不再需要的MCP服务资源
pub async fn start_schedule_task() {
    info!("Start the MCP service status check scheduled task");

    // 创建一个tokio定时器，每20秒执行一次
    let mut interval = interval(Duration::from_secs(20));

    // 使用原子布尔值来跟踪任务是否正在执行
    let is_running = Arc::new(AtomicBool::new(false));

    // 启动一个新的异步任务
    tokio::spawn(async move {
        loop {
            // 等待下一个时间点
            interval.tick().await;

            // 检查是否有任务正在执行
            if is_running.load(Ordering::SeqCst) {
                warn!(
                    "The last MCP service status check task has not been completed and this execution will be skipped."
                );
                continue;
            }

            // 标记任务开始执行
            is_running.store(true, Ordering::SeqCst);

            // 执行MCP服务状态检查
            debug!("Perform periodic checks on MCP service status...");

            // 在一个新的任务中执行检查，这样可以捕获任何异常
            let is_running_clone = is_running.clone();
            tokio::spawn(async move {
                // 执行检查任务
                match tokio::time::timeout(
                    Duration::from_secs(10), // 设置超时时间为10秒，小于间隔时间
                    schedule_check_mcp_live(),
                )
                .await
                {
                    Ok(_) => {
                        debug!("MCP service status check completed");
                    }
                    Err(_) => {
                        warn!("MCP service status check task timed out");
                    }
                }

                // 无论成功还是失败，都标记任务已完成
                is_running_clone.store(false, Ordering::SeqCst);
            });
        }
    });

    info!("MCP service status check scheduled task has been started");
}
