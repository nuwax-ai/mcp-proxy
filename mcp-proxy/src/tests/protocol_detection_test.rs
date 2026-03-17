//! Protocol Detection Integration Tests
//!
//! Tests protocol detection (SSE vs Streamable HTTP) against real MCP services.
//!
//! # Running the tests
//!
//! ```bash
//! # Run the zimage streamable HTTP test (requires DASHSCOPE_API_KEY)
//! DASHSCOPE_API_KEY=sk-xxx cargo test -p mcp-stdio-proxy test_zimage_protocol_detection -- --ignored --nocapture
//!
//! # Run the howtocook SSE test
//! cargo test -p mcp-stdio-proxy test_howtocook_sse_protocol_detection -- --ignored --nocapture
//!
//! # Run all protocol detection network tests
//! DASHSCOPE_API_KEY=sk-xxx cargo test -p mcp-stdio-proxy protocol_detection_test -- --ignored --nocapture
//!
//! # Run with debug logging
//! DASHSCOPE_API_KEY=sk-xxx RUST_LOG=debug cargo test -p mcp-stdio-proxy protocol_detection_test -- --ignored --nocapture
//! ```

use crate::model::{McpJsonServerParameters, McpProtocol, McpServerConfig};

/// The howtocook SSE MCP JSON config (SSE protocol, 测试环境密钥)
const HOWTOCOOK_MCP_JSON: &str = r#"{
  "mcpServers": {
    "howtocook-跳跳糖": {
      "type": "sse",
      "url": "https://testagent.xspaceagi.com/api/mcp/sse?ak=ak-27b83516dfd4417a82f764fe3e859a6e"
    }
  }
}"#;

/// Build zimage MCP JSON config with Authorization token from environment variable
///
/// 环境变量: `DASHSCOPE_API_KEY`
/// 运行网络测试前需设置: `export DASHSCOPE_API_KEY=sk-xxx`
fn build_zimage_mcp_json(api_key: &str) -> String {
    format!(
        r#"{{
  "mcpServers": {{
    "zimage": {{
      "type": "streamableHttp",
      "description": "Z-Image-Turbo 图像生成模型",
      "isActive": true,
      "name": "阿里云百炼_Z Image 图像生成",
      "baseUrl": "https://dashscope.aliyuncs.com/api/v1/mcps/zimage/mcp",
      "headers": {{
        "Authorization": "Bearer {}"
      }}
    }}
  }}
}}"#,
        api_key
    )
}

/// 用于本地解析测试的占位密钥（不需要真实密钥）
const ZIMAGE_TEST_PLACEHOLDER_KEY: &str = "test-placeholder-key";

// ==================== 本地解析测试（无网络） ====================

#[test]
fn test_zimage_config_type_parsed_as_stream() {
    let zimage_json = build_zimage_mcp_json(ZIMAGE_TEST_PLACEHOLDER_KEY);
    let params = McpJsonServerParameters::from(zimage_json);
    let config = params.try_get_first_mcp_server().unwrap();

    match config {
        McpServerConfig::Url(url_config) => {
            // 1. 原始 type 字段值
            assert_eq!(url_config.r#type, Some("streamableHttp".to_string()));

            // 2. get_protocol_type() 返回 Some，且 is_streamable
            let protocol_type = url_config.get_protocol_type();
            assert!(protocol_type.is_some(), "streamableHttp should be recognized");
            assert!(protocol_type.as_ref().unwrap().is_streamable());

            // 3. 转换为 McpProtocol::Stream
            assert_eq!(
                protocol_type.unwrap().to_mcp_protocol(),
                McpProtocol::Stream
            );

            // 4. FromStr 解析也能识别 "streamableHttp"
            assert_eq!(
                "streamableHttp".parse::<McpProtocol>(),
                Ok(McpProtocol::Stream)
            );

            // 5. URL 正确解析
            assert_eq!(
                url_config.get_url(),
                "https://dashscope.aliyuncs.com/api/v1/mcps/zimage/mcp"
            );

            // 6. headers 包含 Authorization
            let headers = url_config.headers.as_ref().unwrap();
            assert!(headers.contains_key("Authorization"));
            assert_eq!(
                headers["Authorization"],
                format!("Bearer {}", ZIMAGE_TEST_PLACEHOLDER_KEY)
            );
        }
        McpServerConfig::Command(_) => panic!("Expected URL config"),
    }
}

// ==================== 网络探测测试（需要网络，默认 ignore） ====================

/// Test: SSE detector should return false for a Streamable HTTP service
///
/// 使用真实 zimage 服务验证：
/// - is_sse_with_headers → false（不是 SSE）
/// - is_streamable_http_with_headers → true（是 Streamable HTTP）
/// - detect_mcp_protocol_with_headers → Stream
///
/// 需要设置环境变量: `DASHSCOPE_API_KEY`
#[tokio::test]
#[ignore] // 需要网络访问 + DASHSCOPE_API_KEY 环境变量
async fn test_zimage_protocol_detection() {
    let api_key = std::env::var("DASHSCOPE_API_KEY")
        .expect("需要设置 DASHSCOPE_API_KEY 环境变量才能运行此测试");
    let zimage_json = build_zimage_mcp_json(&api_key);
    let params = McpJsonServerParameters::from(zimage_json);
    let config = params.try_get_first_mcp_server().unwrap();

    let (url, headers) = match config {
        McpServerConfig::Url(url_config) => {
            let url = url_config.get_url().to_string();
            let headers = url_config.headers.clone().unwrap_or_default();
            (url, headers)
        }
        _ => panic!("Expected URL config"),
    };

    println!("=== 协议探测测试: {} ===", url);

    // 1. SSE 探测应返回 false
    println!("\n--- SSE 探测 ---");
    let is_sse = mcp_sse_proxy::is_sse_with_headers(&url, Some(&headers)).await;
    println!("is_sse_with_headers = {}", is_sse);
    assert!(!is_sse, "Streamable HTTP 服务不应被识别为 SSE");

    // 2. Streamable HTTP 探测应返回 true
    println!("\n--- Streamable HTTP 探测 ---");
    let is_stream =
        mcp_streamable_proxy::is_streamable_http_with_headers(&url, Some(&headers)).await;
    println!("is_streamable_http_with_headers = {}", is_stream);
    assert!(
        is_stream,
        "zimage 服务应被识别为 Streamable HTTP（需要传递 Authorization header）"
    );

    // 3. 综合探测应返回 Stream
    println!("\n--- 综合协议探测 ---");
    let detected = crate::server::detect_mcp_protocol_with_headers(&url, Some(&headers))
        .await
        .unwrap();
    println!("detect_mcp_protocol_with_headers = {:?}", detected);
    assert_eq!(
        detected,
        McpProtocol::Stream,
        "综合探测应返回 Stream 协议"
    );

    println!("\n=== 所有探测结果正确 ===");
}

// ==================== howtocook SSE 本地解析测试 ====================

#[test]
fn test_howtocook_config_type_parsed_as_sse() {
    let params = McpJsonServerParameters::from(HOWTOCOOK_MCP_JSON.to_string());
    let config = params.try_get_first_mcp_server().unwrap();

    match config {
        McpServerConfig::Url(url_config) => {
            // 1. 原始 type 字段值
            assert_eq!(url_config.r#type, Some("sse".to_string()));

            // 2. get_protocol_type() 返回 Some，且不是 streamable
            let protocol_type = url_config.get_protocol_type();
            assert!(protocol_type.is_some(), "sse should be recognized");
            assert!(!protocol_type.as_ref().unwrap().is_streamable());

            // 3. 转换为 McpProtocol::Sse
            assert_eq!(
                protocol_type.unwrap().to_mcp_protocol(),
                McpProtocol::Sse
            );

            // 4. FromStr 解析也能识别 "sse"
            assert_eq!("sse".parse::<McpProtocol>(), Ok(McpProtocol::Sse));
            assert_eq!("SSE".parse::<McpProtocol>(), Ok(McpProtocol::Sse));

            // 5. URL 正确解析（使用 url 字段而非 baseUrl）
            assert_eq!(
                url_config.get_url(),
                "https://testagent.xspaceagi.com/api/mcp/sse?ak=ak-27b83516dfd4417a82f764fe3e859a6e"
            );

            // 6. 无 headers
            assert!(url_config.headers.is_none());
        }
        McpServerConfig::Command(_) => panic!("Expected URL config"),
    }
}

// ==================== howtocook SSE 网络探测测试 ====================

/// Test: SSE detector should return true for a real SSE service
///
/// 使用真实 howtocook SSE 服务验证：
/// - is_sse_with_headers → true（是 SSE，应探测到 event: endpoint）
/// - detect_mcp_protocol_with_headers → Sse
#[tokio::test]
#[ignore] // 需要网络访问，使用 --ignored 运行
async fn test_howtocook_sse_protocol_detection() {
    let params = McpJsonServerParameters::from(HOWTOCOOK_MCP_JSON.to_string());
    let config = params.try_get_first_mcp_server().unwrap();

    let url = match config {
        McpServerConfig::Url(url_config) => url_config.get_url().to_string(),
        _ => panic!("Expected URL config"),
    };

    println!("=== SSE 协议探测测试: {} ===", url);

    // 1. SSE 探测应返回 true（该服务是真实 MCP SSE，会发送 event: endpoint）
    println!("\n--- SSE 探测 ---");
    let is_sse = mcp_sse_proxy::is_sse(&url).await;
    println!("is_sse = {}", is_sse);
    assert!(is_sse, "howtocook SSE 服务应被识别为 SSE（发现 event: endpoint）");

    // 2. Streamable HTTP 探测应返回 false（SSE 服务不应被识别为 Streamable HTTP）
    println!("\n--- Streamable HTTP 探测 ---");
    let is_stream = mcp_streamable_proxy::is_streamable_http(&url).await;
    println!("is_streamable_http = {}", is_stream);
    assert!(
        !is_stream,
        "SSE 服务不应被识别为 Streamable HTTP"
    );

    // 3. 综合探测应返回 Sse
    println!("\n--- 综合协议探测 ---");
    let detected = crate::server::detect_mcp_protocol(&url).await.unwrap();
    println!("detect_mcp_protocol = {:?}", detected);
    assert_eq!(
        detected,
        McpProtocol::Sse,
        "综合探测应返回 Sse 协议"
    );

    println!("\n=== SSE 探测结果正确 ===");
}

/// Test: protocol detection without headers should still default to Stream
///
/// 不传 Authorization header，探测可能失败，但应兜底为 Stream
#[tokio::test]
#[ignore] // 需要网络访问
async fn test_zimage_detection_without_headers() {
    let url = "https://dashscope.aliyuncs.com/api/v1/mcps/zimage/mcp";

    println!("=== 无 header 探测测试: {} ===", url);

    // SSE 探测应返回 false
    let is_sse = mcp_sse_proxy::is_sse(url).await;
    println!("is_sse = {}", is_sse);
    assert!(!is_sse);

    // 综合探测应兜底为 Stream
    let detected = crate::server::detect_mcp_protocol(url).await.unwrap();
    println!("detect_mcp_protocol = {:?}", detected);
    assert_eq!(
        detected,
        McpProtocol::Stream,
        "无 header 时应兜底为 Stream"
    );

    println!("=== 无 header 探测结果正确 ===");
}
