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
use tokio::io::AsyncBufReadExt;
use tracing::{debug, error, info, warn};

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

    // 诊断日志：记录将要传递给子进程的关键环境信息
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let user_env_path = config.env.as_ref().and_then(|e| e.get("PATH").cloned());
    let effective_path = user_env_path.as_deref().unwrap_or(&inherited_path);

    // 🔧 Windows 特殊处理：检测并转换 .cmd/.bat 文件避免弹窗
    // 如果用户配置了 npm 全局安装的 MCP 服务（如 npx some-server 或 some-server.cmd），
    // 直接运行会弹 CMD 窗口。这里尝试转换
    check_windows_command(&config.command);

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

    let mut wrapped_cmd = CommandWrap::with_new(&config.command, |command| {
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

    // 使用传入的 listener 启动 HTTP 服务器

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
