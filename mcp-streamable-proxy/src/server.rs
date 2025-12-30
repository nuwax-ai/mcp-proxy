//! Streamable HTTP server implementation
//!
//! This module provides the HTTP server that uses ProxyAwareSessionManager
//! for stateful session management with backend version control.

use std::sync::Arc;
use anyhow::{Result, bail};
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo},
    transport::{
        TokioChildProcess,
        streamable_http_server::{StreamableHttpService, StreamableHttpServerConfig},
    },
};
use tokio::process::Command;
pub use mcp_common::McpServiceConfig;

use crate::{ProxyHandler, ProxyAwareSessionManager};

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
                bail!("服务器错误: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            if !quiet {
                eprintln!("\n🛑 收到退出信号，正在关闭...");
            }
        }
    }

    Ok(())
}
