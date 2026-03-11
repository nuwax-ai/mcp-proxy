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

    // 诊断日志：记录将要传递给子进程的关键环境信息
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let user_env_path = env.get("PATH").cloned();
    let effective_path = user_env_path.as_deref().unwrap_or(&inherited_path);
    tracing::debug!("[子进程环境][{}] 命令: {} {:?}", name, command, cmd_args);
    tracing::debug!("[子进程环境][{}] 继承 PATH: {}", name, inherited_path);
    if let Some(ref user_path) = user_env_path {
        tracing::debug!("[子进程环境][{}] 用户覆盖 PATH: {}", name, user_path);
    }
    tracing::debug!("[子进程环境][{}] 生效 PATH: {}", name, effective_path);
    {
        let non_path_keys: Vec<&String> = env.keys().filter(|k| *k != "PATH").collect();
        if !non_path_keys.is_empty() {
            tracing::debug!(
                "[子进程环境][{}] 用户自定义环境变量: {:?}",
                name,
                non_path_keys
            );
        }
    }

    // 打印进程继承的镜像源环境变量，便于诊断镜像是否生效
    for key in &["UV_INDEX_URL", "PIP_INDEX_URL", "npm_config_registry"] {
        if let Ok(val) = std::env::var(key) {
            tracing::debug!("[子进程环境][{}] {}={}", name, key, val);
        }
    }

    // 使用 process-wrap 创建子进程命令（跨平台进程清理）
    // process-wrap 会自动处理进程组（Unix）或 Job Object（Windows）
    // 并且在 Drop 时自动清理子进程树
    let mut wrapped_cmd = CommandWrap::with_new(command, |cmd| {
        cmd.args(&cmd_args);

        // ✅ 修复：先继承当前进程的所有环境变量（确保 PATH 等系统变量传递到孙进程）
        // 这样当子服务动态执行 npm/npx 时能正确找到命令
        // 注意：用户提供的 env 会在后面覆盖同名变量，优先级更高
        for (key, value) in std::env::vars_os() {
            if let (Ok(key_str), Ok(value_str)) = (key.into_string(), value.into_string()) {
                cmd.env(key_str, value_str);
            }
        }

        // 然后覆盖/添加用户配置的环境变量（用户配置优先级更高）
        for (k, v) in &env {
            cmd.env(k, v);
        }
    });
    // Unix: 创建进程组，支持 killpg 清理整个进程树
    #[cfg(unix)]
    wrapped_cmd.wrap(ProcessGroup::leader());
    // Windows: 使用 Job Object 管理进程树，并隐藏控制台窗口
    // 使用 CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP 确保孙进程也不弹出窗口
    #[cfg(windows)]
    {
        use process_wrap::tokio::CreationFlags;
        use windows::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        wrapped_cmd.wrap(CreationFlags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP));
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

/// 创建 ClientInfo（使用 rmcp 1.1.0 类型）
fn create_client_info() -> ClientInfo {
    let capabilities = ClientCapabilities::builder()
        .enable_experimental()
        .enable_roots()
        .enable_roots_list_changed()
        .enable_sampling()
        .build();
    ClientInfo::new(
        capabilities,
        Implementation::new("mcp-proxy-cli", env!("CARGO_PKG_VERSION")),
    )
}
