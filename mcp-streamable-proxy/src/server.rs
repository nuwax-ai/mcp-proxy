//! Streamable HTTP server implementation
//!
//! This module provides the HTTP server that uses ProxyAwareSessionManager
//! for stateful session management with backend version control.

use anyhow::{Result, bail};
use mcp_common::{McpServiceConfig, check_windows_command, wrap_process_v9};
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use std::process::Stdio;
use std::sync::Arc;
use tracing::{error, info, warn};

// 进程组管理（跨平台子进程清理）
// process-wrap 9.0 使用 CommandWrap 而不是 TokioCommandWrap
use process_wrap::tokio::{CommandWrap, KillOnDrop};

use crate::{ProxyAwareSessionManager, ProxyHandler};

/// 从配置启动 Streamable HTTP 服务器
///
/// # Features
///
/// - **Stateful Mode**: `stateful_mode: true` 支持 session 管理和服务端推送
/// - **Version Control**: 自动检测后端重连，使旧 session 失效
/// - **Full Lifecycle**: 自动创建子进程、连接、handler、服务器
///
/// # Arguments
///
/// * `config` - MCP 服务配置
/// * `std_listener` - 预先绑定的 TCP 监听器（端口在重试循环前绑定，保证端口占用）
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_stream_server_from_config(
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

    let mut wrapped_cmd = CommandWrap::with_new(&config.command, |command| {
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
    wrap_process_v9!(wrapped_cmd);

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
    let capabilities = ClientCapabilities::builder()
        .enable_experimental()
        .enable_roots()
        .enable_roots_list_changed()
        .enable_sampling()
        .build();
    let client_info = ClientInfo::new(
        capabilities,
        rmcp::model::Implementation::new("mcp-streamable-proxy-server", env!("CARGO_PKG_VERSION")),
    );

    // 4. 连接到子进程
    let client = client_info.serve(tokio_process).await?;

    // 记录子进程启动到日志文件
    info!(
        "[Subprocess startup] Streamable HTTP - Service name: {}, Command: {} {:?}",
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
                    warn!(
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

    // 5. 创建 ProxyHandler
    let proxy_handler = if let Some(tool_filter) = config.tool_filter {
        ProxyHandler::with_tool_filter(client, config.name.clone(), tool_filter)
    } else {
        ProxyHandler::with_mcp_id(client, config.name.clone())
    };

    // 6. 启动服务器（使用预绑定的 listener）
    let listener = tokio::net::TcpListener::from_std(std_listener.try_clone()?)?;
    run_stream_server(proxy_handler, listener, quiet).await
}

/// Run Streamable HTTP server with ProxyAwareSessionManager
///
/// # Features
///
/// - **Stateful Mode**: `stateful_mode: true` 支持 session 管理和服务端推送
/// - **Version Control**: 自动检测后端重连，使旧 session 失效
/// - **Hot Swap**: 支持后端连接热替换
///
/// # Arguments
///
/// * `proxy_handler` - ProxyHandler 实例（包含后端版本控制）
/// * `listener` - 已绑定的 tokio TcpListener
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_stream_server(
    proxy_handler: ProxyHandler,
    listener: tokio::net::TcpListener,
    quiet: bool,
) -> Result<()> {
    let bind_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let mcp_id = proxy_handler.mcp_id().to_string();

    // 记录服务启动到日志文件
    info!(
        "[HTTP service startup] Streamable HTTP service startup - Address: {}, MCP ID: {}",
        bind_addr, mcp_id
    );

    if !quiet {
        eprintln!("📡 Streamable HTTP service startup: http://{}", bind_addr);
        eprintln!("💡 MCP client can be used directly: http://{}", bind_addr);
        eprintln!("✨ Feature: stateful_mode (session management + server push)");
        eprintln!("🔄 Backend version control: Enable (automatically handles reconnections)");
        eprintln!("💡 Press Ctrl+C to stop the service");
    }

    // 包装 handler 为 Arc，供 SessionManager 和 service factory 共享
    let handler = Arc::new(proxy_handler);

    // 创建自定义 SessionManager（带版本控制）
    let session_manager = ProxyAwareSessionManager::new(handler.clone());

    // 创建 Streamable HTTP 服务
    // service factory 每次请求都会调用，返回 handler 的克隆
    let handler_for_service = handler.clone();
    let service = StreamableHttpService::new(
        move || Ok((*handler_for_service).clone()),
        session_manager.into(), // 转换为 Arc<dyn SessionManager>
        StreamableHttpServerConfig {
            stateful_mode: true, // 关键：启用有状态模式
            ..Default::default()
        },
    );

    // Streamable HTTP 直接在根路径提供服务
    let router = axum::Router::new().fallback_service(service);

    // 使用传入的 listener 启动 HTTP 服务器

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                error!(
                    "[HTTP Service Error] Streamable HTTP Server Error - MCP ID: {}, Error: {}",
                    mcp_id, e
                );
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!(
                "[HTTP service shutdown] Received exit signal, closing Streamable HTTP service - MCP ID: {}",
                mcp_id
            );
            if !quiet {
                eprintln!("\\n🛑 Received exit signal, closing...");
            }
        }
    }

    Ok(())
}
