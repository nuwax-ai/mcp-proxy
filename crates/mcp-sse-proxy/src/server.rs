//! SSE server implementation
//!
//! This module provides the SSE server using rmcp 0.10's stable SSE transport.

use anyhow::{Result, bail};
use mcp_common::{McpServiceConfig, check_windows_command, wrap_process_v8};
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo, ProtocolVersion},
    transport::{
        TokioChildProcess,
        sse_server::{SseServer, SseServerConfig},
    },
};
use std::process::Stdio;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

// 进程组管理（跨平台子进程清理）
use process_wrap::tokio::{KillOnDrop, TokioCommandWrap};

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
/// * `std_listener` - 预先绑定的 TCP 监听器（端口在重试循环前绑定，保证端口占用）
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_sse_server_from_config(
    config: McpServiceConfig,
    std_listener: &std::net::TcpListener,
    quiet: bool,
) -> Result<()> {
    // 1. 使用 process-wrap 创建子进程命令（跨平台进程清理）
    // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
    // 并且在 Drop 时自动清理子进程树
    // 子进程默认继承父进程的所有环境变量

    // 🔧 Windows 特殊处理：检测并转换 .cmd/.bat 文件避免弹窗
    // 如果用户配置了 npm 全局安装的 MCP 服务（如 npx some-server 或 some-server.cmd），
    // 直接运行会弹 CMD 窗口。这里尝试转换
    check_windows_command(&config.command);

    info!(
        "[Subprocess][{}] Command: {} {:?}",
        config.name,
        config.command,
        config.args.as_ref().unwrap_or(&vec![])
    );

    let mut wrapped_cmd = TokioCommandWrap::with_new(&config.command, |command| {
        if let Some(ref cmd_args) = config.args {
            command.args(cmd_args);
        }
        // 子进程默认继承父进程的所有环境变量
        // 设置 MCP JSON 配置中的环境变量（会覆盖继承的同名变量）
        if let Some(ref env_vars) = config.env {
            for (k, v) in env_vars {
                command.env(k, v);
            }
        }
    });

    // 应用平台特定的进程包装（Unix: ProcessGroup, Windows: CREATE_NO_WINDOW + JobObject）
    wrap_process_v8!(wrapped_cmd);

    // 所有平台: Drop 时自动清理进程
    wrapped_cmd.wrap(KillOnDrop);

    // 2. 启动子进程（rmcp 的 TokioChildProcess 已经支持 process-wrap）
    //    使用 builder 模式捕获 stderr，便于诊断子 MCP 服务初始化失败
    let (tokio_process, child_stderr) = TokioChildProcess::builder(wrapped_cmd)
        .stderr(Stdio::piped())
        .spawn()?;

    // 启动 stderr 日志读取任务
    if let Some(stderr_pipe) = child_stderr {
        mcp_common::spawn_stderr_reader(stderr_pipe, config.name.clone());
    }

    // 3. 创建客户端信息
    let client_info = ClientInfo {
        protocol_version: ProtocolVersion::V_2024_11_05,
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

    // 记录子进程启动到日志文件
    info!(
        "[Subprocess startup] SSE - Service name: {}, Command: {} {:?}",
        config.name,
        config.command,
        config.args.as_ref().unwrap_or(&vec![])
    );

    if !quiet {
        eprintln!("✅ The child process has been started");

        // 获取并打印工具列表
        match client.list_tools(None).await {
            Ok(tools_result) => {
                let tools = &tools_result.tools;
                if tools.is_empty() {
                    info!(
                        "[Tool list] Tool list is empty - Service name: {}",
                        config.name
                    );
                    eprintln!("⚠️Tool list is empty");
                } else {
                    info!(
                        "[Tool list] Service name: {}, Number of tools: {}",
                        config.name,
                        tools.len()
                    );
                    eprintln!("🔧 Available tools ({}):", tools.len());
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
                        eprintln!("... and {} other tools", tools.len() - 10);
                    }
                }
            }
            Err(e) => {
                error!(
                    "[Tool List] Failed to obtain tool list - Service name: {}, Error: {}",
                    config.name, e
                );
                eprintln!("⚠️ Failed to obtain tool list: {}", e);
            }
        }
    } else {
        // 即使静默模式也记录日志
        match client.list_tools(None).await {
            Ok(tools_result) => {
                info!(
                    "[Tool list] Service name: {}, Number of tools: {}",
                    config.name,
                    tools_result.tools.len()
                );
            }
            Err(e) => {
                error!(
                    "[Tool List] Failed to obtain tool list - Service name: {}, Error: {}",
                    config.name, e
                );
            }
        }
    }

    // 5. 创建 SseHandler
    let sse_handler = if let Some(tool_filter) = config.tool_filter {
        SseHandler::with_tool_filter(client, config.name.clone(), tool_filter)
    } else {
        SseHandler::with_mcp_id(client, config.name.clone())
    };

    // 6. 启动服务器（使用预绑定的 listener）
    let listener = tokio::net::TcpListener::from_std(std_listener.try_clone()?)?;
    run_sse_server(sse_handler, listener, quiet).await
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
/// * `listener` - 已绑定的 tokio TcpListener
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_sse_server(
    sse_handler: SseHandler,
    listener: tokio::net::TcpListener,
    quiet: bool,
) -> Result<()> {
    // 从 listener 获取绑定地址
    let bind_addr = listener.local_addr()?;
    let bind_addr_str = bind_addr.to_string();

    // 默认的 SSE 和消息路径
    let sse_path = "/sse".to_string();
    let message_path = "/message".to_string();
    let mcp_id = sse_handler.mcp_id().to_string();

    // 记录服务启动到日志文件
    info!(
        "[HTTP service startup] SSE service startup - Address: {}, MCP ID: {}, SSE endpoint: {}, Message endpoint: {}",
        bind_addr_str, mcp_id, sse_path, message_path
    );

    if !quiet {
        eprintln!("📡 SSE service startup: http://{}", bind_addr_str);
        eprintln!("SSE endpoint: http://{}{}", bind_addr_str, sse_path);
        eprintln!("Message endpoint: http://{}{}", bind_addr_str, message_path);
        eprintln!(
            "💡 MCP client can be used directly: http://{} (automatic redirection)",
            bind_addr_str
        );
        eprintln!("🔄 Backend hot replacement: enabled");
        eprintln!("💡 Press Ctrl+C to stop the service");
    }

    // 配置 SSE 服务器
    let config = SseServerConfig {
        bind: bind_addr,
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
                _ => (
                    axum::http::StatusCode::METHOD_NOT_ALLOWED,
                    [("Allow", "GET, POST".to_string())],
                    "Method not allowed".to_string(),
                ),
            }
        }
    };

    // 合并路由：SSE 路由 + 根路径兼容处理
    let router = sse_router.fallback(fallback_handler);

    // 使用传入的 listener 启动 HTTP 服务器

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                error!(
                    "[HTTP Service Error] SSE Server Error - MCP ID: {}, Error: {}",
                    mcp_id, e
                );
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!(
                "[HTTP service shutdown] Received exit signal, closing SSE service - MCP ID: {}",
                mcp_id
            );
            if !quiet {
                eprintln!("\\n🛑 Received exit signal, closing...");
            }
            ct.cancel();
        }
    }

    Ok(())
}
