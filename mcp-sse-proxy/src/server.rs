//! SSE server implementation
//!
//! This module provides the SSE server using rmcp 0.10's stable SSE transport.

use anyhow::{Result, bail};
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        sse_server::{SseServer, SseServerConfig},
    },
};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
pub use mcp_common::McpServiceConfig;

use crate::SseHandler;

/// 从配置启动 SSE 服务器
///
/// # Features
///
/// - **SSE Protocol**: 使用稳定的 rmcp 0.10 SSE 实现
/// - **Hot Swap**: 支持后端连接热替换
/// - **Full Lifecycle**: 自动创建子进程、连接、handler、服务器
///
/// # Arguments
///
/// * `config` - MCP 服务配置
/// * `bind_addr` - 绑定地址，例如 "127.0.0.1:3001"
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_sse_server_from_config(
    config: McpServiceConfig,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    // 1. 创建子进程命令
    let mut command = Command::new(&config.command);

    if let Some(ref cmd_args) = config.args {
        command.args(cmd_args);
    }

    if let Some(ref env_vars) = config.env {
        for (k, v) in env_vars {
            command.env(k, v);
        }
    }

    // 2. 启动子进程
    let tokio_process = TokioChildProcess::new(command)?;

    // 3. 创建客户端信息
    let client_info = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        ..Default::default()
    };

    // 4. 连接到子进程
    let client = client_info.serve(tokio_process).await?;

    if !quiet {
        eprintln!("✅ 子进程已启动");

        // 获取并打印工具列表
        match client.list_tools(None).await {
            Ok(tools_result) => {
                let tools = &tools_result.tools;
                if tools.is_empty() {
                    eprintln!("⚠️  工具列表为空");
                } else {
                    eprintln!("🔧 可用工具 ({} 个):", tools.len());
                    for tool in tools.iter().take(10) {
                        let desc = tool.description.as_deref().unwrap_or("无描述");
                        let desc_short = if desc.len() > 50 {
                            format!("{}...", &desc[..50])
                        } else {
                            desc.to_string()
                        };
                        eprintln!("   - {} : {}", tool.name, desc_short);
                    }
                    if tools.len() > 10 {
                        eprintln!("   ... 和 {} 个其他工具", tools.len() - 10);
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  获取工具列表失败: {}", e);
            }
        }
    }

    // 5. 创建 SseHandler
    let sse_handler = if let Some(tool_filter) = config.tool_filter {
        SseHandler::with_tool_filter(client, config.name.clone(), tool_filter)
    } else {
        SseHandler::with_mcp_id(client, config.name.clone())
    };

    // 6. 启动服务器
    run_sse_server(sse_handler, bind_addr, quiet).await
}

/// Run SSE server with rmcp 0.10
///
/// # Features
///
/// - **SSE Protocol**: 使用稳定的 rmcp 0.10 SSE 实现
/// - **Hot Swap**: 支持后端连接热替换（通过 SseHandler）
/// - **Keep-Alive**: 15秒心跳，防止连接被中间代理关闭
///
/// # Arguments
///
/// * `sse_handler` - SseHandler 实例（包含热替换逻辑）
/// * `bind_addr` - 绑定地址，例如 "127.0.0.1:3001"
/// * `quiet` - 静默模式，不输出启动信息
///
/// # Example
///
/// ```no_run
/// use mcp_sse_proxy::{SseHandler, run_sse_server};
///
/// # async fn example() -> anyhow::Result<()> {
/// let handler = SseHandler::new_disconnected(
///     "test-mcp".to_string(),
///     Default::default(),
///     Default::default(),
/// );
///
/// run_sse_server(handler, "127.0.0.1:3001", false).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run_sse_server(
    sse_handler: SseHandler,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    // 默认的 SSE 和消息路径
    let sse_path = "/sse".to_string();
    let message_path = "/message".to_string();

    if !quiet {
        eprintln!("📡 SSE 服务启动: http://{}", bind_addr);
        eprintln!("   SSE 端点: http://{}{}", bind_addr, sse_path);
        eprintln!("   消息端点: http://{}{}", bind_addr, message_path);
        eprintln!("💡 MCP 客户端可直接使用: http://{} （自动重定向）", bind_addr);
        eprintln!("🔄 后端热替换: 启用");
        eprintln!("💡 按 Ctrl+C 停止服务");
    }

    // 配置 SSE 服务器
    let config = SseServerConfig {
        bind: bind_addr.parse()?,
        sse_path: sse_path.clone(),
        post_path: message_path.clone(),
        ct: CancellationToken::new(),
        sse_keep_alive: Some(std::time::Duration::from_secs(15)),
    };

    // 创建 SSE 服务器
    let (sse_server, sse_router) = SseServer::new(config);
    let ct = sse_server.with_service(move || sse_handler.clone());

    // 根路径兼容处理器 - 自动重定向到正确的端点
    let sse_path_for_fallback = sse_path.clone();
    let message_path_for_fallback = message_path.clone();

    let fallback_handler = move |method: axum::http::Method, headers: axum::http::HeaderMap| {
        let sse_path = sse_path_for_fallback.clone();
        let message_path = message_path_for_fallback.clone();
        async move {
            match method {
                axum::http::Method::GET => {
                    // 检查 Accept 头
                    let accept = headers
                        .get("accept")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");

                    if accept.contains("text/event-stream") {
                        // SSE 请求，重定向到 /sse
                        (
                            axum::http::StatusCode::TEMPORARY_REDIRECT,
                            [("Location", sse_path)],
                            "Redirecting to SSE endpoint".to_string(),
                        )
                    } else {
                        // 普通 GET 请求，返回服务信息
                        (
                            axum::http::StatusCode::OK,
                            [("Content-Type", "application/json".to_string())],
                            serde_json::json!({
                                "status": "running",
                                "protocol": "SSE",
                                "endpoints": {
                                    "sse": sse_path,
                                    "message": message_path
                                },
                                "usage": "Connect your MCP client to this URL or the SSE endpoint directly"
                            }).to_string(),
                        )
                    }
                }
                axum::http::Method::POST => {
                    // POST 请求，重定向到 /message
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("Location", message_path)],
                        "Redirecting to message endpoint".to_string(),
                    )
                }
                _ => {
                    (
                        axum::http::StatusCode::METHOD_NOT_ALLOWED,
                        [("Allow", "GET, POST".to_string())],
                        "Method not allowed".to_string(),
                    )
                }
            }
        }
    };

    // 合并路由：SSE 路由 + 根路径兼容处理
    let router = sse_router.fallback(fallback_handler);

    // 启动 HTTP 服务器
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
            ct.cancel();
        }
    }

    Ok(())
}
