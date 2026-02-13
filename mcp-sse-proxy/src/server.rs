//! SSE server implementation
//!
//! This module provides the SSE server using rmcp 0.10's stable SSE transport.

use anyhow::{Result, bail};
pub use mcp_common::McpServiceConfig;
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo, ProtocolVersion},
    transport::{
        TokioChildProcess,
        sse_server::{SseServer, SseServerConfig},
    },
};
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

// 进程组管理（跨平台子进程清理）
use process_wrap::tokio::{KillOnDrop, TokioCommandWrap};

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[cfg(windows)]
use process_wrap::tokio::{CreationFlags, JobObject};

#[cfg(windows)]
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

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

    // 诊断日志：记录将要传递给子进程的关键环境信息
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let user_env_path = config.env.as_ref().and_then(|e| e.get("PATH").cloned());
    let effective_path = user_env_path.as_deref().unwrap_or(&inherited_path);
    info!(
        "[子进程环境][{}] 命令: {} {:?}",
        config.name,
        config.command,
        config.args.as_ref().unwrap_or(&vec![])
    );
    debug!(
        "[子进程环境][{}] 继承 PATH: {}",
        config.name, inherited_path
    );
    if let Some(ref user_path) = user_env_path {
        info!(
            "[子进程环境][{}] 用户覆盖 PATH: {}",
            config.name, user_path
        );
    }
    info!(
        "[子进程环境][{}] 生效 PATH: {}",
        config.name, effective_path
    );
    if let Some(ref env_vars) = config.env {
        let non_path_keys: Vec<&String> = env_vars.keys().filter(|k| *k != "PATH").collect();
        if !non_path_keys.is_empty() {
            info!(
                "[子进程环境][{}] 用户自定义环境变量: {:?}",
                config.name, non_path_keys
            );
        }
    }

    let mut wrapped_cmd = TokioCommandWrap::with_new(&config.command, |command| {
        if let Some(ref cmd_args) = config.args {
            command.args(cmd_args);
        }

        // ✅ 修复：先继承当前进程的所有环境变量（确保 PATH 等系统变量传递到孙进程）
        // 这样当子服务动态执行 npm/npx 时能正确找到命令
        // 注意：用户提供的 env 会在后面覆盖同名变量，优先级更高
        for (key, value) in std::env::vars_os() {
            if let (Ok(key_str), Ok(value_str)) = (key.into_string(), value.into_string()) {
                command.env(key_str, value_str);
            }
        }

        // 然后覆盖/添加用户配置的环境变量（用户配置优先级更高）
        if let Some(ref env_vars) = config.env {
            for (k, v) in env_vars {
                command.env(k, v);
            }
        }
    });
    // Unix: 创建进程组，支持 killpg 清理整个进程树
    #[cfg(unix)]
    wrapped_cmd.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    {
        wrapped_cmd.wrap(CreationFlags(CREATE_NO_WINDOW));
        wrapped_cmd.wrap(JobObject);
    }
    // 所有平台: Drop 时自动清理进程
    wrapped_cmd.wrap(KillOnDrop);

    // 2. 启动子进程（rmcp 的 TokioChildProcess 已经支持 process-wrap）
    //    使用 builder 模式捕获 stderr，便于诊断子 MCP 服务初始化失败
    let (tokio_process, child_stderr) = TokioChildProcess::builder(wrapped_cmd)
        .stderr(Stdio::piped())
        .spawn()?;

    // 启动 stderr 日志读取任务
    if let Some(stderr_pipe) = child_stderr {
        let service_name = config.name.clone();
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr_pipe);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            warn!("[子进程 stderr][{}] {}", service_name, trimmed);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
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
        "[子进程启动] SSE - 服务名: {}, 命令: {} {:?}",
        config.name,
        config.command,
        config.args.as_ref().unwrap_or(&vec![])
    );

    if !quiet {
        eprintln!("✅ 子进程已启动");

        // 获取并打印工具列表
        match client.list_tools(None).await {
            Ok(tools_result) => {
                let tools = &tools_result.tools;
                if tools.is_empty() {
                    warn!("[工具列表] 工具列表为空 - 服务名: {}", config.name);
                    eprintln!("⚠️  工具列表为空");
                } else {
                    info!(
                        "[工具列表] 服务名: {}, 工具数量: {}",
                        config.name,
                        tools.len()
                    );
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
                error!(
                    "[工具列表] 获取工具列表失败 - 服务名: {}, 错误: {}",
                    config.name, e
                );
                eprintln!("⚠️  获取工具列表失败: {}", e);
            }
        }
    } else {
        // 即使静默模式也记录日志
        match client.list_tools(None).await {
            Ok(tools_result) => {
                info!(
                    "[工具列表] 服务名: {}, 工具数量: {}",
                    config.name,
                    tools_result.tools.len()
                );
            }
            Err(e) => {
                error!(
                    "[工具列表] 获取工具列表失败 - 服务名: {}, 错误: {}",
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
pub async fn run_sse_server(sse_handler: SseHandler, listener: tokio::net::TcpListener, quiet: bool) -> Result<()> {
    // 从 listener 获取绑定地址
    let bind_addr = listener.local_addr()?;
    let bind_addr_str = bind_addr.to_string();

    // 默认的 SSE 和消息路径
    let sse_path = "/sse".to_string();
    let message_path = "/message".to_string();
    let mcp_id = sse_handler.mcp_id().to_string();

    // 记录服务启动到日志文件
    info!(
        "[HTTP服务启动] SSE 服务启动 - 地址: {}, MCP ID: {}, SSE端点: {}, 消息端点: {}",
        bind_addr_str, mcp_id, sse_path, message_path
    );

    if !quiet {
        eprintln!("📡 SSE 服务启动: http://{}", bind_addr_str);
        eprintln!("   SSE 端点: http://{}{}", bind_addr_str, sse_path);
        eprintln!("   消息端点: http://{}{}", bind_addr_str, message_path);
        eprintln!(
            "💡 MCP 客户端可直接使用: http://{} （自动重定向）",
            bind_addr_str
        );
        eprintln!("🔄 后端热替换: 启用");
        eprintln!("💡 按 Ctrl+C 停止服务");
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
                    "[HTTP服务错误] SSE 服务器错误 - MCP ID: {}, 错误: {}",
                    mcp_id, e
                );
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!(
                "[HTTP服务关闭] 收到退出信号，正在关闭 SSE 服务 - MCP ID: {}",
                mcp_id
            );
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
            ct.cancel();
        }
    }

    Ok(())
}
