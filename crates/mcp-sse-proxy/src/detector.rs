use futures::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;
use tracing::{debug, info};

/// Reusable reqwest client for SSE probing (connection pooling + consistent timeouts).
static SSE_PROBE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10)) // overall request timeout (incl. response headers + body)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new()) // 自定义 builder 失败时回退默认 client，绝不 panic
});

/// Detect if a URL supports the SSE (Server-Sent Events) MCP protocol
///
/// Convenience wrapper around [`is_sse_with_headers`] that passes no custom headers.
pub async fn is_sse(url: &str) -> bool {
    is_sse_with_headers(url, None).await
}

/// Build candidate URLs for SSE probing, handling query parameters correctly.
///
/// # Examples
///
/// - `http://host/sse?token=xxx` → [`http://host/sse?token=xxx`]
/// - `http://host/mcp?token=xxx` → [`http://host/mcp/sse?token=xxx`, `http://host/mcp?token=xxx`]
/// - `http://host/mcp` → [`http://host/mcp/sse`, `http://host/mcp`]
fn build_candidate_urls(url: &str) -> Vec<String> {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return vec![url.to_string()], // fallback for unparseable URLs
    };

    let path = parsed.path();
    let path_trimmed = path.trim_end_matches('/');

    // If path already ends with /sse, just return the original URL
    if path_trimmed.ends_with("/sse") {
        return vec![url.to_string()];
    }

    // Build candidate: append /sse to the path, preserve query + fragment
    let mut sse_url = parsed.clone();
    sse_url.set_path(&format!("{}/sse", path_trimmed));

    vec![sse_url.to_string(), url.to_string()]
}

/// Detect if a URL supports the MCP SSE protocol, with optional custom headers
///
/// MCP SSE protocol has a unique characteristic: upon GET connection to the SSE
/// endpoint, the server sends an `event: endpoint` event containing the URL for
/// POSTing messages. This `endpoint` event is exclusive to MCP SSE and never appears
/// in Streamable HTTP, making it the definitive distinguishing feature.
///
/// # Detection logic
///
/// 1. Send GET request with `Accept: text/event-stream`
/// 2. Verify response Content-Type is `text/event-stream`
/// 3. Read the first few events from the SSE stream
/// 4. If an `event: endpoint` is found → confirmed MCP SSE
///
/// # Candidate URLs
///
/// - If URL ends with `/sse`, try it as-is
/// - Otherwise, try `{url}/sse` first (MCP SSE convention), then the original URL
///
/// # Arguments
///
/// * `url` - The URL to test
/// * `custom_headers` - Optional custom headers (e.g., Authorization)
///
/// # Returns
///
/// Returns `true` if the URL supports MCP SSE protocol, `false` otherwise.
pub async fn is_sse_with_headers(
    url: &str,
    custom_headers: Option<&HashMap<String, String>>,
) -> bool {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));

    // Merge custom headers
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

    let has_custom_headers = custom_headers.is_some_and(|h| !h.is_empty());

    // Build candidate URLs using proper URL parsing (handles query params correctly).
    //
    // - If URL path already ends with `/sse`, probe as-is (preserving query params).
    // - Otherwise, try `{path}/sse{?query}` first (MCP SSE convention), then original URL.
    let candidates = build_candidate_urls(url);

    for probe_url in &candidates {
        debug!("SSE probe: trying {}", probe_url);
        // 单层超时：SSE_PROBE_CLIENT 已配置 connect_timeout(5s) + timeout(10s)，
        // 后者覆盖「连接 + 响应头 + body 读取」全流程，故无需外层 tokio::time::timeout。
        match probe_sse_endpoint(&SSE_PROBE_CLIENT, probe_url, &headers).await {
            ProbeResult::Success => {
                debug!(
                    "SSE probe: confirmed {} is MCP SSE (endpoint event found)",
                    probe_url
                );
                return true;
            }
            failure => {
                let reason = failure.reason();
                if has_custom_headers {
                    info!(
                        "SSE probe: {} is NOT MCP SSE — {} (custom headers were used)",
                        probe_url, reason
                    );
                } else {
                    debug!("SSE probe: {} is NOT MCP SSE — {}", probe_url, reason);
                }
            }
        }
    }

    false
}

/// Result of probing a single URL for MCP SSE protocol.
enum ProbeResult {
    Success,
    HttpError(u16),
    WrongContentType(Option<String>),
    NoEndpointEvent,
    ConnectionFailed,
}

impl ProbeResult {
    fn reason(&self) -> String {
        match self {
            ProbeResult::Success => "success".to_string(),
            ProbeResult::HttpError(code) => format!("HTTP {}", code),
            ProbeResult::WrongContentType(ct) => {
                format!("Content-Type is {:?} (expected text/event-stream)", ct)
            }
            ProbeResult::NoEndpointEvent => {
                "no endpoint event found in first 4KB of stream".to_string()
            }
            ProbeResult::ConnectionFailed => "connection failed".to_string(),
        }
    }
}

/// Probe a single URL for MCP SSE protocol.
async fn probe_sse_endpoint(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
) -> ProbeResult {
    let response = match client.get(url).headers(headers.clone()).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!("SSE probe: connection to {} failed: {}", url, e);
            return ProbeResult::ConnectionFailed;
        }
    };

    let status = response.status();
    if !status.is_success() {
        let code = status.as_u16();
        debug!("SSE probe: {} returned HTTP {}", url, code);
        return ProbeResult::HttpError(code);
    }

    // Verify Content-Type is text/event-stream
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let is_event_stream = content_type
        .as_ref()
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if !is_event_stream {
        debug!(
            "SSE probe: {} Content-Type is {:?}, expected text/event-stream",
            url, content_type
        );
        return ProbeResult::WrongContentType(content_type);
    }

    // Read the SSE stream looking for "event: endpoint"
    if read_sse_for_endpoint_event(response).await {
        ProbeResult::Success
    } else {
        ProbeResult::NoEndpointEvent
    }
}

/// Check whether a byte buffer contains the MCP SSE `endpoint` event signature.
///
/// Matches both `event: endpoint` (standard) and `event:endpoint` (no space) so MCP SSE is
/// identified regardless of server formatting. Operates on raw bytes to handle non-UTF8 and
/// cross-chunk boundaries correctly.
fn contains_endpoint_event(buffer: &[u8]) -> bool {
    const PATTERN_WITH_SPACE: &[u8] = b"event: endpoint";
    const PATTERN_NO_SPACE: &[u8] = b"event:endpoint";
    // 两个模式长度不同（15 vs 14），须分别以各自长度做 windows，否则较短模式会被漏检
    buffer
        .windows(PATTERN_WITH_SPACE.len())
        .any(|w| w == PATTERN_WITH_SPACE)
        || buffer
            .windows(PATTERN_NO_SPACE.len())
            .any(|w| w == PATTERN_NO_SPACE)
}

/// Read SSE stream and check for the `endpoint` event.
///
/// MCP SSE servers send `event: endpoint\ndata: <message_url>\n\n` as the
/// first event after connection. This event is never sent by Streamable HTTP.
async fn read_sse_for_endpoint_event(response: reqwest::Response) -> bool {
    let mut stream = response.bytes_stream();
    let mut buffer: Vec<u8> = Vec::new();
    const MAX_BYTES: usize = 4096;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                buffer.extend_from_slice(&bytes);

                if contains_endpoint_event(&buffer) {
                    return true;
                }

                // Don't read too much — if endpoint event hasn't appeared
                // in the first 4KB, it's not MCP SSE
                if buffer.len() > MAX_BYTES {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains_endpoint_event_with_space() {
        // 标准 MCP SSE endpoint 事件（带空格）
        let data = b"event: endpoint\ndata: /messages?session_id=abc123\n\n";
        assert!(contains_endpoint_event(data));
    }

    #[test]
    fn test_contains_endpoint_event_without_space() {
        // 无空格变体（旧实现用较长模式的长度做 windows，导致较短模式被漏检，已修复）
        let data = b"event:endpoint\ndata: /messages?session_id=abc123\n\n";
        assert!(contains_endpoint_event(data));
    }

    #[test]
    fn test_contains_endpoint_event_absent() {
        // 普通 SSE 流，不含 endpoint 事件
        let data = b"data: some data\n\n";
        assert!(!contains_endpoint_event(data));
    }

    #[tokio::test]
    async fn test_is_sse_nonexistent_server() {
        let result = is_sse("http://localhost:99999/mcp").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_is_sse_with_headers_no_panic() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer test-token".to_string());
        let result = is_sse_with_headers("http://localhost:99999/mcp", Some(&headers)).await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_candidate_urls_with_sse_suffix() {
        let result = is_sse("http://localhost:99999/sse").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_candidate_urls_with_trailing_slash() {
        let result = is_sse("http://localhost:99999/mcp/").await;
        assert!(!result);
    }

    #[test]
    fn test_build_candidate_urls_with_query_params() {
        // URL with query params, path doesn't end with /sse
        let candidates = build_candidate_urls("http://host/mcp?token=abc123");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "http://host/mcp/sse?token=abc123");
        assert_eq!(candidates[1], "http://host/mcp?token=abc123");
    }

    #[test]
    fn test_build_candidate_urls_sse_with_query_params() {
        // URL already ends with /sse, has query params
        let candidates = build_candidate_urls("http://host/sse?token=abc123&other=1");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "http://host/sse?token=abc123&other=1");
    }

    #[test]
    fn test_build_candidate_urls_sse_no_query() {
        // URL already ends with /sse, no query params
        let candidates = build_candidate_urls("http://host/sse");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "http://host/sse");
    }

    #[test]
    fn test_build_candidate_urls_no_query() {
        // URL without /sse, no query params
        let candidates = build_candidate_urls("http://host/mcp");
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], "http://host/mcp/sse");
        assert_eq!(candidates[1], "http://host/mcp");
    }
}
