//! 健康检查命令
//!
//! 通过建立 MCP 连接验证服务是否健康

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use mcp_common::{McpClientConfig, t};
use mcp_sse_proxy::SseClientConnection;
use mcp_streamable_proxy::StreamClientConnection;

use crate::client::protocol::detect_mcp_protocol;
use crate::client::proxy_server::ProxyProtocol;
use crate::client::support::HealthArgs;
use crate::model::McpProtocol;

/// 健康检查结果
struct HealthCheckResult {
    /// 工具数量
    tool_count: usize,
    /// 服务器名称
    server_name: Option<String>,
    /// 服务器版本
    server_version: Option<String>,
}

/// 运行健康检查命令
///
/// 通过真正建立 MCP 连接来验证服务是否健康。
/// 成功返回 Ok(())，失败返回 Err。
pub async fn run_health_command(args: HealthArgs, quiet: bool) -> Result<()> {
    if !quiet {
        eprintln!("{}", t!("cli.health.checking", url = &args.url));
    }

    // 1. 确定协议类型
    let protocol = match &args.protocol {
        Some(p) => {
            let proto = proxy_protocol_to_mcp_protocol(p.clone());
            if !quiet {
                eprintln!("{}", t!("cli.health.using_protocol", protocol = protocol_display_name(&proto)));
            }
            proto
        }
        None => {
            if !quiet {
                eprintln!("{}", t!("cli.health.detecting_protocol"));
            }
            let proto = detect_mcp_protocol(&args.url).await?;
            if !quiet {
                eprintln!("{}", t!("cli.health.detected_protocol", protocol = protocol_display_name(&proto)));
            }
            proto
        }
    };

    // 2. 检查协议类型是否支持
    if protocol == McpProtocol::Stdio {
        bail!("{}", t!("cli.health.stdio_not_supported"));
    }

    // 3. 构建配置
    let config = build_config(&args);

    // 4. 尝试连接（带超时）
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(args.timeout),
        connect_and_verify(&config, protocol.clone()),
    )
    .await;
    let elapsed = start.elapsed();

    // 5. 输出结果
    match result {
        Ok(Ok(health_result)) => {
            if !quiet {
                eprintln!("{}", t!("cli.health.healthy"));
                eprintln!("   协议: {}", protocol_display_name(&protocol));
                eprintln!("   工具数量: {}", health_result.tool_count);
                eprintln!("   响应时间: {}ms", elapsed.as_millis());
                if let (Some(name), Some(version)) =
                    (&health_result.server_name, &health_result.server_version)
                {
                    eprintln!("   服务器: {} v{}", name, version);
                } else if let Some(name) = &health_result.server_name {
                    eprintln!("   服务器: {}", name);
                }
            }
            Ok(())
        }
        Ok(Err(e)) => {
            if !quiet {
                eprintln!("{}", t!("cli.health.unhealthy"));
                eprintln!("   错误: {}", e);
                eprintln!("   响应时间: {}ms", elapsed.as_millis());
            }
            Err(anyhow::anyhow!("health check failed: {}", e))
        }
        Err(_) => {
            if !quiet {
                eprintln!("{}", t!("cli.health.unhealthy"));
                eprintln!("   错误: 连接超时 ({}s)", args.timeout);
            }
            Err(anyhow::anyhow!("health check timeout"))
        }
    }
}

/// 获取协议的显示名称
fn protocol_display_name(protocol: &McpProtocol) -> &'static str {
    match protocol {
        McpProtocol::Sse => "SSE",
        McpProtocol::Stream => "Streamable HTTP",
        McpProtocol::Stdio => "Stdio",
    }
}

/// 将 ProxyProtocol 转换为 McpProtocol
fn proxy_protocol_to_mcp_protocol(p: ProxyProtocol) -> McpProtocol {
    match p {
        ProxyProtocol::Sse => McpProtocol::Sse,
        ProxyProtocol::Stream => McpProtocol::Stream,
    }
}

/// 构建 MCP 客户端配置
fn build_config(args: &HealthArgs) -> McpClientConfig {
    let mut config = McpClientConfig::new(&args.url);

    // 添加认证 header
    if let Some(auth) = &args.auth {
        config = config.with_header("Authorization", auth);
    }

    // 添加自定义 headers
    for (key, value) in &args.header {
        config = config.with_header(key, value);
    }

    // 设置超时
    config = config.with_connect_timeout(Duration::from_secs(args.timeout));
    config = config.with_read_timeout(Duration::from_secs(args.timeout));

    config
}

/// 尝试连接并验证服务
async fn connect_and_verify(
    config: &McpClientConfig,
    protocol: McpProtocol,
) -> Result<HealthCheckResult> {
    match protocol {
        McpProtocol::Sse => {
            let conn = SseClientConnection::connect(config.clone()).await?;

            // 获取工具列表
            let tools = conn.list_tools().await?;
            let tool_count = tools.len();

            // 获取服务器信息
            let (server_name, server_version) = conn
                .peer_info()
                .map(|info| {
                    (
                        Some(info.server_info.name.clone()),
                        Some(info.server_info.version.clone()),
                    )
                })
                .unwrap_or((None, None));

            Ok(HealthCheckResult {
                tool_count,
                server_name,
                server_version,
            })
        }
        McpProtocol::Stream => {
            let conn = StreamClientConnection::connect(config.clone()).await?;

            // 获取工具列表
            let tools = conn.list_tools().await?;
            let tool_count = tools.len();

            // 获取服务器信息
            let (server_name, server_version) = conn
                .peer_info()
                .map(|info| {
                    (
                        Some(info.server_info.name.clone()),
                        Some(info.server_info.version.clone()),
                    )
                })
                .unwrap_or((None, None));

            Ok(HealthCheckResult {
                tool_count,
                server_name,
                server_version,
            })
        }
        McpProtocol::Stdio => {
            // 不应该到达这里，因为前面已经检查过了
            bail!("stdio protocol is not supported for health check")
        }
    }
}
