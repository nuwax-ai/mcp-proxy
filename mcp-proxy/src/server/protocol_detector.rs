use anyhow::Result;
use log::{debug, info};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};

use crate::model::McpProtocol;

/// 自动检测 MCP 服务的协议类型
///
/// 通过发送探测请求来判断服务支持的协议：
/// 1. 先尝试 Streamable HTTP 协议（发送带有特定 Accept 头的请求）
/// 2. 如果失败，尝试 SSE 协议
pub async fn detect_mcp_protocol(url: &str) -> Result<McpProtocol> {
    info!("开始自动检测 MCP 服务协议: {}", url);

    // 首先尝试 Streamable HTTP 协议
    if is_streamable_http(url).await {
        info!("检测到 Streamable HTTP 协议: {}", url);
        return Ok(McpProtocol::Stream);
    }

    // 然后尝试 SSE 协议
    if is_sse_protocol(url).await {
        info!("检测到 SSE 协议: {}", url);
        return Ok(McpProtocol::Sse);
    }

    // 如果都不支持，默认返回 SSE（向后兼容）
    info!("无法确定协议类型，默认使用 SSE 协议: {}", url);
    Ok(McpProtocol::Sse)
}

/// 检测是否为 Streamable HTTP 协议
///
/// Streamable HTTP 协议的特征：
/// - 需要 Accept: application/json, text/event-stream 头
/// - 返回 200 OK 或 406 Not Acceptable（如果缺少正确的 Accept 头）
/// - 响应头包含 content-type: text/event-stream 或 application/json
async fn is_streamable_http(url: &str) -> bool {
    debug!("尝试检测 Streamable HTTP 协议: {}", url);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            debug!("创建 HTTP 客户端失败: {}", e);
            return false;
        }
    };

    // 构造一个简单的探测请求
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // 发送一个简单的 ping 或 initialize 请求
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "probe",
        "method": "ping",
        "params": {}
    });

    match client.post(url).headers(headers).json(&body).send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers();

            debug!("Streamable HTTP 探测响应状态: {}", status);
            debug!("响应头: {:?}", headers);

            // 检查响应头中是否包含 mcp-session-id（Streamable HTTP 的特征）
            if headers.contains_key("mcp-session-id") {
                debug!("发现 mcp-session-id 头，确认为 Streamable HTTP 协议");
                return true;
            }

            // 检查 content-type
            if let Some(content_type) = headers.get(CONTENT_TYPE) {
                if let Ok(ct) = content_type.to_str() {
                    debug!("Content-Type: {}", ct);
                    // Streamable HTTP 可能返回 text/event-stream 或 application/json
                    if ct.contains("text/event-stream") || ct.contains("application/json") {
                        // 进一步检查是否为 Streamable HTTP（而不是普通的 JSON API）
                        // 如果状态码是 200 且有正确的 content-type，很可能是 Streamable HTTP
                        if status.is_success() {
                            debug!("响应成功且 Content-Type 匹配，可能是 Streamable HTTP");
                            return true;
                        }
                    }
                }
            }

            // 如果返回 406 Not Acceptable，说明服务器期望特定的 Accept 头
            // 这也是 Streamable HTTP 的一个特征
            if status == reqwest::StatusCode::NOT_ACCEPTABLE {
                debug!("收到 406 Not Acceptable，可能是 Streamable HTTP 协议");
                return true;
            }

            false
        }
        Err(e) => {
            debug!("Streamable HTTP 探测失败: {}", e);
            false
        }
    }
}

/// 检测是否为 SSE 协议
///
/// SSE 协议的特征：
/// - 通常是 GET 请求到特定的 SSE 端点
/// - 响应头包含 content-type: text/event-stream
/// - 连接保持打开状态
async fn is_sse_protocol(url: &str) -> bool {
    debug!("尝试检测 SSE 协议: {}", url);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            debug!("创建 HTTP 客户端失败: {}", e);
            return false;
        }
    };

    // SSE 通常使用 GET 请求
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));

    match client.get(url).headers(headers).send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response.headers();

            debug!("SSE 探测响应状态: {}", status);

            // 检查 content-type 是否为 text/event-stream
            if let Some(content_type) = headers.get(CONTENT_TYPE) {
                if let Ok(ct) = content_type.to_str() {
                    debug!("Content-Type: {}", ct);
                    if ct.contains("text/event-stream") && status.is_success() {
                        debug!("确认为 SSE 协议");
                        return true;
                    }
                }
            }

            false
        }
        Err(e) => {
            debug!("SSE 探测失败: {}", e);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_protocol() {
        // 这个测试需要实际的服务运行
        // 这里只是示例
        let url = "http://127.0.0.1:8000/mcp";
        match detect_mcp_protocol(url).await {
            Ok(protocol) => {
                println!("检测到的协议: {:?}", protocol);
            }
            Err(e) => {
                println!("检测失败: {}", e);
            }
        }
    }
}
