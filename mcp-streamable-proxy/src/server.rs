//! Streamable HTTP server implementation
//!
//! This module provides the HTTP server that uses ProxyAwareSessionManager
//! for stateful session management with backend version control.

use anyhow::{Result, bail};
pub use mcp_common::McpServiceConfig;
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService},
    },
};
use std::sync::Arc;
use tracing::{error, info, warn};

// 进程组管理（跨平台子进程清理）
// process-wrap 9.0 使用 CommandWrap 而不是 TokioCommandWrap
use process_wrap::tokio::{CommandWrap, KillOnDrop};

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[cfg(windows)]
use process_wrap::tokio::JobObject;

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
/// * `bind_addr` - 绑定地址，例如 "127.0.0.1:3000"
/// * `quiet` - 静默模式，不输出启动信息
pub async fn run_stream_server_from_config(
    config: McpServiceConfig,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    // 1. 使用 process-wrap 创建子进程命令（跨平台进程清理）
    // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
    // 并且在 Drop 时自动清理子进程树
    let mut wrapped_cmd = CommandWrap::with_new(&config.command, |command| {
        if let Some(ref cmd_args) = config.args {
            command.args(cmd_args);
        }
        if let Some(ref env_vars) = config.env {
            for (k, v) in env_vars {
                command.env(k, v);
            }
        }
    });
    // Unix: 创建进程组，支持 killpg 清理整个进程树
    #[cfg(unix)]
    wrapped_cmd.wrap(ProcessGroup::leader());
    // 所有平台: Drop 时自动清理进程
    wrapped_cmd.wrap(KillOnDrop);

    // 2. 启动子进程（rmcp 的 TokioChildProcess 已经支持 process-wrap）
    let tokio_process = TokioChildProcess::new(wrapped_cmd)?;

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

    // 记录子进程启动到日志文件
    info!(
        "[子进程启动] Streamable HTTP - 服务名: {}, 命令: {} {:?}",
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

    // 5. 创建 ProxyHandler
    let proxy_handler = if let Some(tool_filter) = config.tool_filter {
        ProxyHandler::with_tool_filter(client, config.name.clone(), tool_filter)
    } else {
        ProxyHandler::with_mcp_id(client, config.name.clone())
    };

    // 6. 启动服务器
    run_stream_server(proxy_handler, bind_addr, quiet).await
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
/// * `bind_addr` - 绑定地址，例如 "127.0.0.1:3000"
/// * `quiet` - 静默模式，不输出启动信息
///
/// # Example
///
/// ```no_run
/// use mcp_streamable_proxy::{ProxyHandler, run_stream_server};
/// use mcp_common::ToolFilter;
///
/// # async fn example() -> anyhow::Result<()> {
/// let handler = ProxyHandler::new_disconnected(
///     "test-mcp".to_string(),
///     ToolFilter::default(),
///     Default::default(),
/// );
///
/// run_stream_server(handler, "127.0.0.1:3000", false).await?;
/// # Ok(())
/// # }
/// ```
pub async fn run_stream_server(
    proxy_handler: ProxyHandler,
    bind_addr: &str,
    quiet: bool,
) -> Result<()> {
    let mcp_id = proxy_handler.mcp_id().to_string();

    // 记录服务启动到日志文件
    info!(
        "[HTTP服务启动] Streamable HTTP 服务启动 - 地址: {}, MCP ID: {}",
        bind_addr, mcp_id
    );

    if !quiet {
        eprintln!("📡 Streamable HTTP 服务启动: http://{}", bind_addr);
        eprintln!("💡 MCP 客户端可直接使用: http://{}", bind_addr);
        eprintln!("✨ 特性: stateful_mode (会话管理 + 服务端推送)");
        eprintln!("🔄 后端版本控制: 启用 (自动处理重连)");
        eprintln!("💡 按 Ctrl+C 停止服务");
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

    // 启动 HTTP 服务器
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    // 使用 select 处理 Ctrl+C 和服务器
    tokio::select! {
        result = axum::serve(listener, router) => {
            if let Err(e) = result {
                error!(
                    "[HTTP服务错误] Streamable HTTP 服务器错误 - MCP ID: {}, 错误: {}",
                    mcp_id, e
                );
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!(
                "[HTTP服务关闭] 收到退出信号，正在关闭 Streamable HTTP 服务 - MCP ID: {}",
                mcp_id
            );
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
        }
    }

    Ok(())
}
