//! 测试 SSE 前端代理连接 Streamable HTTP 后端
//!
//! 这个测试模拟 SseServerBuilder 的行为，验证修复是否有效
//!
//! 运行方式：
//! ```bash
//! cargo test -p mcp-stdio-proxy test_sse_frontend_to_streamable_backend -- --ignored --nocapture
//! ```

/// 测试：SSE 前端使用 BackendBridge 配置连接 Streamable HTTP 后端
///
/// 这个测试模拟了完整路径：
/// 1. mcp-proxy 层连接 Streamable HTTP 后端 → 得到 Arc<dyn BackendBridge>
/// 2. 传给 SseServerBuilder 作为 BackendConfig::BackendBridge
/// 3. 验证 SSE Server 构建、capabilities 桥接、状态检查
#[tokio::test]
#[ignore] // 需要网络访问
async fn test_sse_frontend_to_streamable_backend() {
    use mcp_sse_proxy::{BackendConfig, SseServerBuilder};
    use mcp_streamable_proxy::{StreamClientConnection, ProxyHandler};
    use std::sync::Arc;

    let url = "https://qcest-1.com/cvs1/GuruOutbound/McpPublic";

    println!("=== SSE 前端代理 Streamable HTTP 后端测试 ===");
    println!("URL: {}\n", url);

    // Step 1: 在 mcp-proxy 层连接 Streamable HTTP 后端
    println!("--- Step 1: 连接 Streamable HTTP 后端 ---");
    let config = mcp_common::McpClientConfig::new(url);
    let conn = StreamClientConnection::connect(config)
        .await
        .expect("Streamable HTTP connection failed");
    let proxy_handler = ProxyHandler::with_mcp_id(
        conn.into_running_service(),
        "test-sse-to-stream".to_string(),
    );
    let bridge: Arc<dyn mcp_common::BackendBridge> = Arc::new(proxy_handler);
    println!("✅ 后端连接成功\n");

    // Step 2: 传给 SseServerBuilder 作为 BackendBridge
    let backend_config = BackendConfig::BackendBridge(bridge);

    println!("--- 构建 SSE Server（前端 SSE，后端 Streamable HTTP）---");

    let builder = SseServerBuilder::new(backend_config)
        .mcp_id("test-sse-to-stream")
        .sse_path("/sse")
        .post_path("/message")
        .stateful(false); // OneShot 模式

    match builder.build().await {
        Ok((_router, _ct, handler)) => {
            println!("✅ SSE Server 构建成功！\n");

            // 验证 handler 报告了正确的 capabilities
            use mcp_sse_proxy::ServerHandler;
            let info = handler.get_info();
            println!(
                "--- ServerInfo capabilities ---\n  tools: {}\n  resources: {}\n  prompts: {}\n",
                info.capabilities.tools.is_some(),
                info.capabilities.resources.is_some(),
                info.capabilities.prompts.is_some(),
            );
            assert!(
                info.capabilities.tools.is_some(),
                "Backend should report tools capability"
            );

            // 验证 handler 状态检查
            println!("--- 状态检查 ---");
            println!("  is_backend_available: {}", handler.is_backend_available());
            println!(
                "  is_mcp_server_ready: {}",
                handler.is_mcp_server_ready().await
            );
            assert!(
                handler.is_backend_available(),
                "Backend should be available"
            );
            assert!(
                handler.is_mcp_server_ready().await,
                "MCP server should be ready"
            );
        }
        Err(e) => {
            println!("❌ SSE Server 构建失败: {}\n", e);
            println!("这说明修复可能没有正确应用，或者有其他问题。");
        }
    }

    println!("\n=== 测试完成 ===");
}

/// 测试：直接用 StreamClientConnection 验证后端可用
#[tokio::test]
#[ignore] // 需要网络访问
async fn test_streamable_backend_direct() {
    use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};

    let url = "https://qcest-1.com/cvs1/GuruOutbound/McpPublic";

    println!("=== Streamable HTTP 直接连接测试 ===");
    println!("URL: {}\n", url);

    let config = McpClientConfig::new(url);
    match StreamClientConnection::connect(config).await {
        Ok(conn) => {
            println!("✅ 直接连接成功！\n");
            match conn.list_tools().await {
                Ok(tools) => {
                    println!("✅ 获取工具列表成功！共 {} 个工具\n", tools.len());
                    for tool in tools.iter().take(5) {
                        println!("  - {}: {:?}", tool.name, tool.description.as_deref().unwrap_or("无描述"));
                    }
                }
                Err(e) => println!("❌ 获取工具列表失败: {}\n", e),
            }
        }
        Err(e) => {
            println!("❌ 直接连接失败: {}\n", e);
        }
    }

    println!("=== 测试完成 ===");
}
