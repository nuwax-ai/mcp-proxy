//! 本地命令模式处理
//!
//! 处理本地命令形式的 MCP 服务（通过子进程）

use anyhow::Result;
use std::collections::HashMap;

use crate::proxy::{StreamProxyHandler, ToolFilter};

// 使用 mcp-streamable-proxy 的类型（rmcp 0.12，process-wrap 9.0）
use mcp_streamable_proxy::{
    ClientCapabilities, ClientInfo, Implementation, ServiceExt, TokioChildProcess, stdio,
};

use crate::client::support::utils::truncate_str;

// 进程组管理（跨平台子进程清理）
use process_wrap::tokio::{CommandWrap, KillOnDrop};

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[cfg(windows)]
use process_wrap::tokio::JobObject;

/// 命令模式执行（本地子进程）
/// 使用 mcp-streamable-proxy（rmcp 0.12）实现 stdio CLI 模式
pub async fn run_command_mode(
    name: &str,
    command: &str,
    cmd_args: Vec<String>,
    env: HashMap<String, String>,
    tool_filter: ToolFilter,
    verbose: bool,
    quiet: bool,
) -> Result<()> {
    tracing::info!("模式: 本地命令模式");
    tracing::info!("命令: {} {:?}", command, cmd_args);
    if !env.is_empty() {
        tracing::debug!("环境变量数量: {}", env.len());
    }

    if !quiet {
        eprintln!("🚀 MCP-Stdio-Proxy: {} (command) → stdio", name);
        eprintln!("   命令: {} {:?}", command, cmd_args);
        if verbose && !env.is_empty() {
            eprintln!("   环境变量: {:?}", env);
        }
    }

    // 显示过滤器配置
    if !quiet && tool_filter.is_enabled() {
        eprintln!("🔧 工具过滤已启用");
    }

    // 使用 process-wrap 创建子进程命令（跨平台进程清理）
    // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
    // 并且在 Drop 时自动清理子进程树
    let mut wrapped_cmd = CommandWrap::with_new(command, |cmd| {
        cmd.args(&cmd_args);
        for (k, v) in &env {
            cmd.env(k, v);
        }
    });
    // Unix: 创建进程组，支持 killpg 清理整个进程树
    #[cfg(unix)]
    wrapped_cmd.wrap(ProcessGroup::leader());
    // Windows: 使用 Job Object 管理进程树，并隐藏控制台窗口
    #[cfg(windows)]
    {
        use process_wrap::CreationFlags;
        // CREATE_NO_WINDOW = 0x08000000
        // 隐藏控制台窗口，避免在 GUI 应用（如 Tauri）中显示 CMD 窗口
        wrapped_cmd.wrap(CreationFlags(0x08000000));
        wrapped_cmd.wrap(JobObject);
    }
    // 所有平台: Drop 时自动清理进程
    wrapped_cmd.wrap(KillOnDrop);

    // 启动子进程
    tracing::debug!("启动子进程...");
    let tokio_process = TokioChildProcess::new(wrapped_cmd)?;

    if !quiet {
        eprintln!("🔗 启动子进程...");
    }

    // 创建 ClientInfo（使用 rmcp 0.12 类型）
    let client_info = create_client_info();

    // 连接到子进程
    let running = client_info.serve(tokio_process).await?;

    if !quiet {
        eprintln!("✅ 子进程已启动，开始代理转换...");

        // 打印工具列表
        match running.list_tools(None).await {
            Ok(tools_result) => {
                let tools = &tools_result.tools;
                if tools.is_empty() {
                    eprintln!("⚠️  工具列表为空 (tools/list 返回 0 个工具)");
                } else {
                    eprintln!("🔧 可用工具 ({} 个):", tools.len());
                    for tool in tools {
                        let desc = tool.description.as_deref().unwrap_or("无描述");
                        let desc_short = truncate_str(desc, 50);
                        eprintln!("   - {} : {}", tool.name, desc_short);
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  获取工具列表失败: {}", e);
            }
        }

        eprintln!("💡 现在可以通过 stdin 发送 JSON-RPC 请求");
    }

    // 使用 StreamProxyHandler + stdio 将本地 MCP 服务透明暴露为 stdio
    let proxy_handler =
        StreamProxyHandler::with_tool_filter(running, name.to_string(), tool_filter);
    let server = proxy_handler.serve(stdio()).await?;

    // 设置 Ctrl+C 信号处理
    tokio::select! {
        result = server.waiting() => {
            result?;
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到 Ctrl+C 信号，正在关闭...");
            // tokio runtime 会清理资源，包括子进程
        }
    }

    Ok(())
}

/// 创建 ClientInfo（使用 rmcp 0.12 类型）
fn create_client_info() -> ClientInfo {
    ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        client_info: Implementation {
            name: "mcp-proxy-cli".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: None,
            website_url: None,
            icons: None,
        },
    }
}
