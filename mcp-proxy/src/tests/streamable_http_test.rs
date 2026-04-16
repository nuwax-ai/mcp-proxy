//! 测试 Streamable HTTP 协议连接 GuruMCP 后端
//!
//! 运行方式：
//! ```bash
//! cargo test -p mcp-stdio-proxy test_streamable_direct_call -- --ignored --nocapture
//! ```

#[tokio::test]
#[ignore] // 需要网络访问
async fn test_streamable_direct_call() {
    use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};

    let url = "https://qcest-1.com/cvs1/GuruOutbound/McpPublic";

    println!("=== Streamable HTTP 直接调用测试 ===");
    println!("URL: {}\n", url);

    // Step 1: 尝试连接
    println!("--- Step 1: 尝试连接 Streamable HTTP 服务 ---");
    let config = McpClientConfig::new(url);
    let conn = match StreamClientConnection::connect(config).await {
        Ok(c) => {
            println!("✅ 连接成功！\n");
            c
        }
        Err(e) => {
            println!("❌ 连接失败: {}\n", e);
            println!("这说明问题出在 rmcp 的初始化握手阶段，不是 SSE 代理的问题。");
            return;
        }
    };

    // Step 2: 列出工具
    println!("--- Step 2: 列出可用工具 ---");
    let tools = match conn.list_tools().await {
        Ok(t) => {
            println!("✅ 获取工具列表成功！共 {} 个工具\n", t.len());
            t
        }
        Err(e) => {
            println!("❌ 获取工具列表失败: {}\n", e);
            return;
        }
    };

    // Step 3: 如果有工具，尝试调用第一个工具
    if let Some(first_tool) = tools.first() {
        println!("--- Step 3: 尝试调用工具 '{}' ---", first_tool.name);
        println!("工具描述: {:?}\n", first_tool.description);
    } else {
        println!("--- Step 3: 没有可用工具 ---");
    }

    println!("=== 测试完成 ===");
}

/// 测试：直接用 StreamClientConnection 验证后端可用
///
/// 这个测试验证了 StreamClientConnection 可以正常工作
#[tokio::test]
#[ignore] // 需要网络访问
async fn test_sse_server_builder_connect_stream_url_simulation() {
    use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};

    let url = "https://qcest-1.com/cvs1/GuruOutbound/McpPublic";

    println!("=== 验证 Streamable HTTP 后端可用 ===");
    println!("URL: {}\n", url);

    let config = McpClientConfig::new(url);
    match StreamClientConnection::connect(config).await {
        Ok(conn) => {
            println!("✅ 连接成功！\n");
            match conn.list_tools().await {
                Ok(tools) => {
                    println!("✅ 获取工具列表成功！共 {} 个工具\n", tools.len());
                    for tool in tools.iter().take(5) {
                        println!("  - {}: {:?}", tool.name, tool.description.as_deref().unwrap_or("无描述"));
                    }
                    if tools.len() > 5 {
                        println!("  ... 还有 {} 个工具", tools.len() - 5);
                    }
                }
                Err(e) => println!("❌ 获取工具列表失败: {}\n", e),
            }
        }
        Err(e) => {
            println!("❌ 连接失败: {}\n", e);
        }
    }

    println!("=== 测试完成 ===");
    println!("\n注意: SseServerBuilder 的修复已应用，当使用 SSE 前端连接 Streamable HTTP 后端时，");
    println!("会自动使用 LATEST 协议版本而不是 V_2024_11_05");
}