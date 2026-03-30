use futures::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;

/// Detect if a URL supports the SSE (Server-Sent Events) MCP protocol
///
/// Convenience wrapper around [`is_sse_with_headers`] that passes no custom headers.
pub async fn is_sse(url: &str) -> bool {
    is_sse_with_headers(url, None).await
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
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

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

    // Build candidate URLs
    let trimmed = url.trim_end_matches('/');
    let candidates: Vec<String> = if trimmed.ends_with("/sse") {
        vec![url.to_string()]
    } else {
        // Try /sse suffix first (MCP SSE convention), then original URL
        vec![format!("{}/sse", trimmed), url.to_string()]
    };

    for probe_url in &candidates {
        debug!("SSE probe: try {}", probe_url);
        match tokio::time::timeout(
            Duration::from_secs(5),
            probe_sse_endpoint(&client, probe_url, &headers),
        )
        .await
        {
            Ok(true) => {
                debug!(
                    "SSE probe: Confirm {} is MCP SSE protocol (discover endpoint event)",
                    probe_url
                );
                return true;
            }
            Ok(false) => {
                debug!("SSE probe: {} is not MCP SSE protocol", probe_url);
            }
            Err(_) => {
                debug!("SSE probe: {} timeout", probe_url);
            }
        }
    }

    false
}

/// Probe a single URL for MCP SSE protocol
///
/// Returns `true` only if the response is `text/event-stream` AND contains
/// an `event: endpoint` SSE event (the MCP SSE distinguishing feature).
async fn probe_sse_endpoint(client: &reqwest::Client, url: &str, headers: &HeaderMap) -> bool {
    let response = match client.get(url).headers(headers.clone()).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };

    if !response.status().is_success() {
        return false;
    }

    // Verify Content-Type is text/event-stream
    let is_event_stream = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if !is_event_stream {
        return false;
    }

    // Read the SSE stream looking for "event: endpoint" — the MCP SSE signature.
    // The endpoint event is sent immediately upon connection, so we only need
    // to read the first few chunks (up to 4KB).
    read_sse_for_endpoint_event(response).await
}

/// Read SSE stream and check for the `endpoint` event
///
/// MCP SSE servers send `event: endpoint\ndata: <message_url>\n\n` as the
/// first event after connection. This event is never sent by Streamable HTTP.
async fn read_sse_for_endpoint_event(response: reqwest::Response) -> bool {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    const MAX_BYTES: usize = 4096;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    buffer.push_str(text);
                }

                // SSE event format: "event: endpoint\n" or "event:endpoint\n"
                if buffer.contains("event: endpoint") || buffer.contains("event:endpoint") {
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
        // URL already ending with /sse should not get /sse appended
        let result = is_sse("http://localhost:99999/sse").await;
        assert!(!result);
    }

    #[tokio::test]
    async fn test_candidate_urls_with_trailing_slash() {
        // Trailing slash should be trimmed before appending /sse
        let result = is_sse("http://localhost:99999/mcp/").await;
        assert!(!result);
    }
}
