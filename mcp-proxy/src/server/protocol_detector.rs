use anyhow::Result;
use log::{debug, info};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue};

use crate::model::McpProtocol;
use rmcp::model::{
    ClientCapabilities, ClientRequest, Implementation, InitializeRequestParam, ProtocolVersion,
    RequestId,
};

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
/// - 支持 POST 请求，响应 JSON-RPC 格式
/// - 对 initialize 请求返回有效响应
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

    // 构造一个标准的 MCP initialize 请求（使用 rmcp 类型）
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // 使用 rmcp 的类型构造 initialize 请求
    let init_request = rmcp::model::ClientRequest::InitializeRequest(rmcp::model::Request::new(
        InitializeRequestParam {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "mcp-proxy-detector".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                icons: None,
                website_url: None,
            },
        },
    ));

    // 序列化为 JSON-RPC 消息
    let body = rmcp::model::ClientJsonRpcMessage::request(init_request, RequestId::Number(1));

    match client.post(url).headers(headers).json(&body).send().await {
        Ok(response) => {
            let status = response.status();
            let resp_headers = response.headers().clone();

            debug!("Streamable HTTP 探测响应状态: {}", status);
            debug!("响应头: {:?}", resp_headers);

            // 检查响应头中是否包含 mcp-session-id（Streamable HTTP 的特征）
            if resp_headers.contains_key("mcp-session-id") {
                debug!("发现 mcp-session-id 头，确认为 Streamable HTTP 协议");
                return true;
            }

            // 检查 Content-Type
            // 关键区别：SSE 协议使用 GET 请求，Streamable HTTP 使用 POST 请求
            // 如果 POST 请求返回 text/event-stream，说明是 Streamable HTTP（支持流式响应）
            if let Some(content_type) = resp_headers.get(CONTENT_TYPE) {
                if let Ok(ct) = content_type.to_str() {
                    debug!("Content-Type: {}", ct);
                    if ct.contains("text/event-stream") && status.is_success() {
                        debug!("POST 请求返回 SSE 流，确认为 Streamable HTTP 协议");
                        return true;
                    }
                }
            }

            // 检查响应是否为 JSON-RPC 格式
            // 注意：即使状态码不是 2xx，只要响应体是有效的 JSON-RPC 格式，也说明是 Streamable HTTP
            if let Ok(json) = response.json::<serde_json::Value>().await {
                debug!("响应内容: {:?}", json);
                // JSON-RPC 2.0 响应必须包含 jsonrpc 字段且值为 "2.0"
                // 这比单独检查 error 字段更严格，避免误判普通 JSON 错误响应
                let is_jsonrpc = json
                    .get("jsonrpc")
                    .and_then(|v| v.as_str())
                    .map(|v| v == "2.0")
                    .unwrap_or(false);

                if is_jsonrpc {
                    debug!("响应为有效 JSON-RPC 2.0 格式，确认为 Streamable HTTP 协议");
                    return true;
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
