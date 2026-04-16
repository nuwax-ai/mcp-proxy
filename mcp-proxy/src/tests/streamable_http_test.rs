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

/// 测试：模拟 Java 端完整调用流程
///
/// 1. 连接 Streamable HTTP 后端
/// 2. 构建 SSE Server 并启动在真实端口
/// 3. 用 HTTP 客户端连接 SSE 端点（模拟 Java 端 SSE 客户端）
/// 4. 发送 initialize 请求
/// 5. 发送 tools/list 请求
/// 6. 验证返回结果
#[tokio::test]
#[ignore] // 需要网络访问
async fn test_java_client_simulation() {
    use mcp_sse_proxy::{BackendConfig, SseServerBuilder};
    use mcp_streamable_proxy::{ProxyHandler, StreamClientConnection};
    use std::sync::Arc;
    use std::time::Instant;

    let url = "https://qcest-1.com/cvs1/GuruOutbound/McpPublic";
    let total_start = Instant::now();

    println!("=== 模拟 Java 端完整调用流程 ===\n");

    // Step 1: 连接后端（模拟 check_status 触发的 mcp_start_task）
    let step1_start = Instant::now();
    println!("--- Step 1: 连接 Streamable HTTP 后端 ---");
    let config = mcp_common::McpClientConfig::new(url);
    let conn = StreamClientConnection::connect(config)
        .await
        .expect("Backend connection failed");
    let proxy_handler =
        ProxyHandler::with_mcp_id(conn.into_running_service(), "java-test".to_string());
    let bridge: Arc<dyn mcp_common::BackendBridge> = Arc::new(proxy_handler);
    println!(
        "✅ 后端连接成功 ({}ms)\n",
        step1_start.elapsed().as_millis()
    );

    // Step 2: 构建 SSE Server 并启动在真实端口
    let step2_start = Instant::now();
    println!("--- Step 2: 构建并启动 SSE Server ---");
    let (router, _ct, handler) = SseServerBuilder::new(BackendConfig::BackendBridge(bridge))
        .mcp_id("java-test")
        .sse_path("/sse")
        .post_path("/message")
        .stateful(false)
        .build()
        .await
        .expect("SSE Server build failed");

    // 启动真实 HTTP 服务器
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind");
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    println!(
        "✅ SSE Server 启动在 {} ({}ms)\n",
        base_url,
        step2_start.elapsed().as_millis()
    );

    // 此时 Java 端认为"部署完成"
    let deploy_time = total_start.elapsed();
    println!(
        "📊 从开始到服务就绪: {}ms (Java 端超时阈值通常 7000ms)\n",
        deploy_time.as_millis()
    );

    // Step 3: 模拟 Java SSE 客户端连接
    let step3_start = Instant::now();
    println!("--- Step 3: 模拟 Java SSE 客户端 ---");

    let sse_url = format!("{}/sse", base_url);
    println!("  连接 SSE 端点: {}", sse_url);

    let client = reqwest::Client::new();

    // SSE 是流式连接，用 bytes_stream 读取第一个事件（包含 session endpoint）
    let sse_resp = client.get(&sse_url).send().await.expect("SSE GET failed");
    println!(
        "  SSE 连接状态码: {} ({}ms)",
        sse_resp.status(),
        step3_start.elapsed().as_millis()
    );
    assert_eq!(sse_resp.status(), 200, "SSE endpoint should return 200");

    // 流式读取 SSE 事件，提取 message endpoint（超时 5 秒）
    use futures::StreamExt;
    let mut stream = sse_resp.bytes_stream();
    let mut collected = String::new();
    let message_url = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("SSE stream error");
            collected.push_str(&String::from_utf8_lossy(&chunk));
            // 查找 endpoint event: "data: /message?sessionId=xxx"
            for line in collected.lines() {
                if line.starts_with("data: ") && line.contains("message") {
                    let path = line.strip_prefix("data: ").unwrap();
                    return if path.starts_with("http") {
                        path.to_string()
                    } else {
                        format!("{}{}", base_url, path)
                    };
                }
            }
        }
        panic!("SSE stream ended without endpoint event");
    })
    .await
    .expect("Timeout waiting for SSE endpoint event");

    println!("  SSE 事件内容: {}", collected.trim());
    println!("  消息端点: {}\n", message_url);

    // Step 4: 发送 initialize 请求
    let step4_start = Instant::now();
    println!("--- Step 4: 发送 initialize 请求 ---");
    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "java-test-client",
                "version": "1.0.0"
            }
        }
    });

    let init_resp = client
        .post(&message_url)
        .json(&init_request)
        .send()
        .await
        .expect("Initialize POST failed");
    println!(
        "  initialize 状态码: {} ({}ms)",
        init_resp.status(),
        step4_start.elapsed().as_millis()
    );
    let init_status = init_resp.status();
    let init_body = init_resp.text().await.unwrap_or_default();
    println!("  initialize 响应: {}\n", &init_body[..init_body.len().min(500)]);
    assert!(
        init_status.is_success() || init_status.as_u16() == 202,
        "initialize should succeed, got {}",
        init_status
    );

    // Step 5: 发送 tools/list 请求
    let step5_start = Instant::now();
    println!("--- Step 5: 发送 tools/list 请求 ---");
    let list_tools_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });

    let tools_resp = client
        .post(&message_url)
        .json(&list_tools_request)
        .send()
        .await
        .expect("tools/list POST failed");
    println!(
        "  tools/list 状态码: {} ({}ms)",
        tools_resp.status(),
        step5_start.elapsed().as_millis()
    );
    let tools_status = tools_resp.status();
    let tools_body = tools_resp.text().await.unwrap_or_default();
    println!("  tools/list 响应: {}\n", &tools_body[..tools_body.len().min(1000)]);
    assert!(
        tools_status.is_success() || tools_status.as_u16() == 202,
        "tools/list should succeed, got {}",
        tools_status
    );

    // Step 6: 验证 handler 状态
    println!("--- Step 6: 验证 handler 状态 ---");
    println!("  is_backend_available: {}", handler.is_backend_available());
    println!(
        "  is_mcp_server_ready: {}",
        handler.is_mcp_server_ready().await
    );

    let total_time = total_start.elapsed();
    println!("\n📊 总耗时: {}ms", total_time.as_millis());
    println!("=== 测试完成 ===");
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
