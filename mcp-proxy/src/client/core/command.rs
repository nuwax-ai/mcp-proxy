//! 本地命令模式处理
//!
//! 处理本地命令形式的 MCP 服务（通过子进程）

use anyhow::Result;
use std::collections::HashMap;
use tokio::process::Command;

use crate::proxy::{ProxyHandler, ToolFilter};

// SSE 模式需要的类型（rmcp 0.10）
use mcp_sse_proxy::{
    ServiceExt as SseServiceExt,
    TokioChildProcess,
    stdio as sse_stdio,
    ClientInfo as SseClientInfo,
    ClientCapabilities as SseClientCapabilities,
    Implementation as SseImplementation,
};

use crate::client::support::utils::truncate_str;

/// 命令模式执行（本地子进程）
/// 使用 SSE 库（rmcp 0.10）的类型
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
    if !quiet {
        if tool_filter.is_enabled() {
            eprintln!("🔧 工具过滤已启用");
        }
    }

    // 创建子进程命令
    let mut cmd = Command::new(command);
    cmd.args(&cmd_args);
    for (k, v) in &env {
        cmd.env(k, v);
    }

    // 启动子进程
    tracing::debug!("启动子进程...");
    let tokio_process = TokioChildProcess::new(cmd)?;

    if !quiet {
        eprintln!("🔗 启动子进程...");
    }

    // 创建 ClientInfo（使用 SSE 库的类型，rmcp 0.10）
    let client_info = create_sse_client_info();

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

    // 使用 ProxyHandler + stdio 将本地 MCP 服务透明暴露为 stdio
    let proxy_handler = ProxyHandler::with_tool_filter(running, name.to_string(), tool_filter);
    let server = proxy_handler.serve(sse_stdio()).await?;
    server.waiting().await?;

    Ok(())
}

/// 创建 SSE 库的 ClientInfo（rmcp 0.10）
fn create_sse_client_info() -> SseClientInfo {
    SseClientInfo {
        protocol_version: Default::default(),
        capabilities: SseClientCapabilities::builder()
            .enable_experimental()
            .enable_roots()
            .enable_roots_list_changed()
            .enable_sampling()
            .build(),
        client_info: SseImplementation {
            name: "mcp-proxy-cli".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: None,
            website_url: None,
            icons: None,
        },
    }
}
