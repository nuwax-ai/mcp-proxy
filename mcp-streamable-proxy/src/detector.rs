use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::time::Duration;

/// Detect if a URL supports the Streamable HTTP protocol (backward compatible, no custom headers)
///
/// This is a convenience wrapper around [`is_streamable_http_with_headers`] that passes no
/// custom headers. See that function for full documentation.
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
    is_streamable_http_with_headers(url, None).await
}

/// Detect if a URL supports the Streamable HTTP protocol, with optional custom headers
///
/// This detection works by sending an MCP Initialize request
/// and checking the response characteristics.
///
/// Custom headers (e.g., `Authorization`) are merged into the detection request,
/// which is essential for MCP services that require authentication.
///
/// # Detection characteristics
///
/// - Presence of `mcp-session-id` response header (Streamable HTTP specific)
/// - Valid JSON-RPC 2.0 response format
/// - POST request returning `text/event-stream` (Streamable HTTP feature)
///
/// # Arguments
///
/// * `url` - The URL to test
/// * `custom_headers` - Optional custom headers to include in the detection request
///
/// # Returns
///
/// Returns `true` if the URL supports Streamable HTTP protocol, `false` otherwise.
pub async fn is_streamable_http_with_headers(
    url: &str,
    custom_headers: Option<&HashMap<String, String>>,
) -> bool {
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

    // Merge custom headers (e.g., Authorization)
    if let Some(custom) = custom_headers {
        for (key, value) in custom {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::try_from(key.as_str()),
                HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }
    }

    // Construct an MCP Initialize request using rmcp 1.1.0 types
    use rmcp::model::{
        ClientCapabilities, ClientRequest, Implementation, InitializeRequestParams,
        ProtocolVersion, Request, RequestId,
    };

    let init_request = ClientRequest::InitializeRequest(Request::new(
        InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new("mcp-proxy-detector", "0.1.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2024_11_05),
    ));

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
    if let Some(content_type) = resp_headers.get(CONTENT_TYPE)
        && let Ok(ct) = content_type.to_str()
        && ct.contains("text/event-stream")
        && status.is_success()
    {
        return true;
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
    async fn test_is_streamable_http_with_headers_backward_compatible() {
        // With None headers should behave identically to is_streamable_http
        let result = is_streamable_http_with_headers("http://localhost:99999/mcp", None).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_is_streamable_http_with_headers_no_panic() {
        // Non-existent server, but validates headers don't cause panics
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer test-token".to_string());
        let result =
            is_streamable_http_with_headers("http://localhost:99999/mcp", Some(&headers)).await;
        assert!(!result);
    }
}
