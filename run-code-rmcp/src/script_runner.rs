use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info};
use rmcp::ServiceExt;
use tokio::io::{stdin, stdout};

use run_code_rmcp::mcp::CodeRunnerService;

/// MCP脚本运行器 - 通过MCP协议执行JavaScript、TypeScript和Python代码
#[derive(Parser)]
#[command(name = "script_runner")]
#[command(author = "MCP Script Runner")]
#[command(version = "0.1.0")]
#[command(about = "通过MCP协议执行JavaScript、TypeScript和Python代码", long_about = None)]
struct Cli {
    /// 启用详细日志输出
    #[arg(short, long)]
    verbose: bool,
}

/// 启动MCP服务器
async fn start_mcp_server(verbose: bool) -> Result<()> {
    if verbose {
        info!("初始化 MCP 服务...");
    }

    // 创建服务实例
    let service = CodeRunnerService;

    // 使用标准输入输出作为传输方式
    let transport = (stdin(), stdout());

    if verbose {
        info!("MCP 服务已启动，等待连接...");
    }

    // 启动服务
    let server = service.serve(transport).await.context("启动MCP服务失败")?;

    // 等待服务结束
    let result = server.waiting().await;

    if verbose {
        match &result {
            Ok(reason) => error!("MCP 服务已停止: {:?}", reason),
            Err(err) => error!("MCP 服务出错: {}", err),
        }
    }

    result.map(|_| ()).map_err(Into::into)
}

#[tokio::main]
async fn main() -> Result<()> {
    // 解析命令行参数
    let cli = Cli::parse();

    // 启动MCP服务
    start_mcp_server(cli.verbose).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{ServiceExt, model::CallToolRequestParams};
    use std::time::Duration;
    use tokio::sync::oneshot;
    use tokio::time::timeout;

    #[tokio::test]
    #[ignore = "MCP client test requires proper service implementation - TODO: fix later"]
    async fn test_start_mcp_server() -> Result<()> {
        // 创建管道，模拟标准输入输出
        let (client_stream, server_stream) = tokio::io::duplex(8192);

        // 分割成读写部分
        let (server_read, server_write) = tokio::io::split(server_stream);
        let (client_read, client_write) = tokio::io::split(client_stream);

        // 使用通道控制服务器生命周期
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        // 在单独的任务中启动服务器
        let server_task = tokio::spawn(async move {
            // 创建服务实例
            let service = CodeRunnerService;

            // 创建服务Future并pin
            let service_fut = service.serve((server_read, server_write));
            tokio::pin!(service_fut);

            // 等待关闭信号或服务器完成
            tokio::select! {
                res = &mut service_fut => {
                    match res {
                        Ok(server) => {
                            match server.waiting().await {
                                Ok(reason) => println!("服务正常结束: {:?}", reason),
                                Err(e) => println!("服务错误: {}", e),
                            }
                        }
                        Err(e) => println!("启动服务失败: {}", e),
                    }
                }
                _ = shutdown_rx => {
                    println!("收到关闭信号");
                }
            }
        });

        // 给服务器一点时间启动
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 使用RMCP客户端SDK连接到服务器
        let client = ().serve((client_read, client_write)).await?;

        // 获取服务器信息
        let server_info = client.peer_info();
        println!("服务器信息: {:?}", server_info);

        // 验证服务器信息
        assert!(
            server_info.unwrap().instructions.is_some(),
            "服务器应该提供说明"
        );

        // 测试执行JavaScript代码
        let js_code =
            "function handler(input) { return {success: true, message: 'JavaScript测试成功'}; }";
        let result = client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "run_javascript".into(),
                arguments: serde_json::json!({
                    "code": js_code,
                })
                .as_object()
                .cloned(),
                task: None,
            })
            .await?;

        // 验证JavaScript结果，这里只简单验证调用成功
        println!("JavaScript执行结果: {:?}", result);

        // 测试执行Python代码
        let py_code =
            "def handler(input):\n    return {'success': True, 'message': 'Python测试成功'}";
        let result = client
            .call_tool(CallToolRequestParams {
                meta: None,
                name: "run_python".into(),
                arguments: serde_json::json!({
                    "code": py_code,
                })
                .as_object()
                .cloned(),
                task: None,
            })
            .await?;

        // 验证Python结果，这里只简单验证调用成功
        println!("Python执行结果: {:?}", result);

        // 关闭客户端
        client.cancel().await?;

        // 发送服务器关闭信号
        let _ = shutdown_tx.send(());

        // 等待服务器任务结束
        let _ = timeout(Duration::from_secs(5), server_task).await;

        Ok(())
    }
}
