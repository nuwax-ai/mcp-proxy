// MCP 协议检测模块
// 用于自动检测远程 MCP 服务的协议类型
// 复用 server 模块的协议检测逻辑

use anyhow::Result;
use std::collections::HashMap;

// 复用 model 中的协议类型定义
pub use crate::model::McpProtocol;

/// 自动检测 MCP 协议类型（带自定义 headers）
///
/// 对于需要 Authorization 等 headers 才能正常响应的 SSE 服务，
/// 必须传入对应的 headers，否则探测器会因收到 401/403 而误判为 Streamable HTTP。
pub async fn detect_mcp_protocol_with_headers(
    url: &str,
    headers: Option<&HashMap<String, String>>,
) -> Result<McpProtocol> {
    crate::server::detect_mcp_protocol_with_headers(url, headers).await
}
