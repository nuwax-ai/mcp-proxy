//! Coze MCP Service Integration Tests
//!
//! This module tests the protocol conversion from Streamable HTTP (backend) to SSE (frontend)
//! when connecting to Coze MCP services.
//!
//! # Configuration
//!
//! These tests require the following environment variables:
//!
//! - `COZE_PLUGIN_ID` - Your Coze plugin ID
//! - `COZE_BEARER_TOKEN` - Your Coze API Bearer token
//!
//! # Running the tests
//!
//! ```bash
//! # Set environment variables and run the test
//! COZE_PLUGIN_ID="your_plugin_id" \
//! COZE_BEARER_TOKEN="your_bearer_token" \
//! cargo test -p mcp-stdio-proxy test_coze_streamable_to_sse_proxy
//!
//! # Or run with logging
//! COZE_PLUGIN_ID="your_plugin_id" \
//! COZE_BEARER_TOKEN="your_bearer_token" \
//! RUST_LOG=debug cargo test -p mcp-stdio-proxy test_coze_streamable_to_sse_proxy
//! ```

use anyhow::Result;
use std::time::Duration;
use tokio::net::TcpListener;

use crate::{
    model::{McpConfig, McpProtocol, McpType},
    proxy::{McpClientConfig, SseClientConnection},
    mcp_start_task,
};

/// Builds Coze MCP configuration from environment variables
fn get_coze_config() -> Result<String> {
    let plugin_id = std::env::var("COZE_PLUGIN_ID")
        .map_err(|_| anyhow::anyhow!("COZE_PLUGIN_ID environment variable not set"))?;
    let bearer_token = std::env::var("COZE_BEARER_TOKEN")
        .map_err(|_| anyhow::anyhow!("COZE_BEARER_TOKEN environment variable not set"))?;

    let config = r#"{
  "mcpServers": {
    "coze_plugin_tianyancha": {
      "url": "https://mcp.coze.cn/v1/plugins/PLUGIN_ID",
      "headers": {
        "Authorization": "Bearer BEARER_TOKEN"
      }
    }
  }
}"#;

    Ok(config
        .replace("PLUGIN_ID", &plugin_id)
        .replace("BEARER_TOKEN", &bearer_token))
}

/// Test: Streamable HTTP backend to SSE frontend protocol conversion
///
/// This test verifies that the mcp-proxy correctly:
/// 1. Configures SSE protocol for the client (frontend)
/// 2. Auto-detects Streamable HTTP protocol for the Coze backend
/// 3. Transparently converts between the two protocols
/// 4. Returns valid tools/list responses
#[tokio::test]
#[ignore] // Mark as ignored since it requires network access
async fn test_coze_streamable_to_sse_proxy() -> Result<()> {
    // Initialize logging for test
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .try_init();

    println!("🧪 Starting Coze MCP test: Streamable HTTP -> SSE conversion");

    // Step 1: Create configuration with SSE client protocol
    // The backend protocol (Streamable HTTP) will be auto-detected
    let coze_config = get_coze_config()?;
    let mcp_config = McpConfig::from_json_with_server(
        "coze_plugin_tianyancha".to_string(),
        coze_config,
        McpType::OneShot,
        McpProtocol::Sse, // SSE frontend (client protocol)
    )?;

    println!("✅ Configuration created with SSE client protocol");

    // Step 2: Start MCP service
    // The proxy will auto-detect the backend protocol and create appropriate routes
    let (router, ct) = mcp_start_task(mcp_config).await?;
    println!("✅ MCP service started");

    // Step 3: Start HTTP server with the router
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    println!("✅ HTTP server listening on 127.0.0.1:{}", port);

    // Spawn server in background
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, router.into_make_service())
            .await
            .expect("Server error");
    });

    // Step 4: Wait for backend to be ready
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("✅ Backend ready");

    // Step 5: Construct SSE endpoint path
    // The SSE server exposes endpoints at /mcp/sse/proxy/{mcp_id}/sse
    let sse_url = format!("http://127.0.0.1:{}/mcp/sse/proxy/coze_plugin_tianyancha/sse", port);

    // Step 6: Connect SSE client
    let client_config = McpClientConfig::new(sse_url.to_string());
    let conn = tokio::time::timeout(
        Duration::from_secs(30),
        SseClientConnection::connect(client_config.clone()),
    )
    .await
    .map_err(|_| anyhow::anyhow!("SSE connection timeout (30s)"))?
    .map_err(|e| anyhow::anyhow!("SSE connection failed: {}", e))?;
    println!("✅ SSE client connected to {}", sse_url);

    // Step 7: Get tools list using the high-level API
    let tools = tokio::time::timeout(
        Duration::from_secs(30),
        conn.list_tools(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("list_tools timeout (30s)"))?
    .map_err(|e| anyhow::anyhow!("list_tools failed: {}", e))?;
    println!("📋 Received tools/list response: {} tools", tools.len());

    // Step 8: Verify response structure
    if tools.is_empty() {
        println!("⚠️  Warning: tools/list returned empty array");
    } else {
        println!("✅ Found {} tools:", tools.len());
        for tool in &tools {
            let desc = tool.description.as_deref().unwrap_or("no description");
            println!("   - {} : {}", tool.name, desc);
        }
    }

    // Step 9: Verify tool structure
    for tool in &tools {
        assert!(!tool.name.is_empty(), "Tool name should not be empty");
        // Description is optional, so we just check the name
        println!("   ✓ Tool '{}' has valid structure", tool.name);
    }

    // Step 10: Cleanup
    ct.cancel();
    server_handle.abort();
    println!("🧹 Cleanup complete");

    Ok(())
}
