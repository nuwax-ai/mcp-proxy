//! Streamable HTTP Protocol Detection
//!
//! This module provides a detection function to determine if a given URL
//! supports the Streamable HTTP MCP protocol.

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::time::Duration;

/// Detects if a URL supports the Streamable HTTP protocol
///
/// This function sends an MCP Initialize request to the URL and checks the response
/// characteristics to determine if it's a Streamable HTTP endpoint:
///
/// - Presence of `mcp-session-id` response header (Streamable HTTP specific)
/// - Valid JSON-RPC 2.0 response format
/// - POST request returning `text/event-stream` (Streamable HTTP characteristic)
///
/// # Arguments
///
/// * `url` - The URL to test
///
/// # Returns
///
/// Returns `true` if the URL supports Streamable HTTP protocol, `false` otherwise.
///
/// # Example
///
/// ```rust,ignore
/// use mcp_streamable_proxy::is_streamable_http;
///
/// if is_streamable_http("http://localhost:8080/mcp").await {
///     println!("Server supports Streamable HTTP");
/// }
/// ```
pub async fn is_streamable_http(url: &str) -> bool {
    // Build HTTP client with timeout
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Construct headers for Streamable HTTP detection
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // Construct an MCP Initialize request using rmcp 0.12 types
    use rmcp::model::{
        ClientCapabilities, ClientRequest, Implementation, InitializeRequestParam, ProtocolVersion,
        Request, RequestId,
    };

    let init_request = ClientRequest::InitializeRequest(Request::new(InitializeRequestParam {
        protocol_version: ProtocolVersion::V_2024_11_05,
        capabilities: ClientCapabilities::default(),
        client_info: Implementation {
            name: "mcp-proxy-detector".to_string(),
            version: "0.1.0".to_string(),
            title: None,
            icons: None,
            website_url: None,
        },
    }));

    // Serialize to JSON-RPC message
    let body = rmcp::model::ClientJsonRpcMessage::request(init_request, RequestId::Number(1));

    // Send POST request and analyze response
    let response = match client.post(url).headers(headers).json(&body).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };

    let status = response.status();
    let resp_headers = response.headers().clone();

    // Check 1: Presence of mcp-session-id header (Streamable HTTP specific)
    if resp_headers.contains_key("mcp-session-id") {
        return true;
    }

    // Check 2: POST request returning text/event-stream (Streamable HTTP feature)
    if let Some(content_type) = resp_headers.get(CONTENT_TYPE) {
        if let Ok(ct) = content_type.to_str() {
            if ct.contains("text/event-stream") && status.is_success() {
                return true;
            }
        }
    }

    // Check 3: Valid JSON-RPC 2.0 response (even if status is not 2xx)
    if let Ok(json) = response.json::<serde_json::Value>().await {
        // JSON-RPC 2.0 response must have jsonrpc: "2.0" field
        let is_jsonrpc = json
            .get("jsonrpc")
            .and_then(|v| v.as_str())
            .map(|v| v == "2.0")
            .unwrap_or(false);

        if is_jsonrpc {
            return true;
        }
    }

    // Check 4: 406 Not Acceptable might indicate Streamable HTTP expecting specific headers
    if status == reqwest::StatusCode::NOT_ACCEPTABLE {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_is_streamable_http_invalid_url() {
        // Invalid URL should return false without panic
        let result = is_streamable_http("not-a-url").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_is_streamable_http_nonexistent_server() {
        // Non-existent server should return false
        let result = is_streamable_http("http://localhost:99999/mcp").await;
        assert!(!result);
    }

    // Note: Real integration tests would require a running MCP server
    // and should be added in separate integration test files
}
