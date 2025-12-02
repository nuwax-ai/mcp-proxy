// MCP 协议检测模块
// 用于自动检测远程 MCP 服务的协议类型
// 复用 server 模块的协议检测逻辑

use anyhow::Result;

// 复用 model 中的协议类型定义
pub use crate::model::McpProtocol;

/// 自动检测 MCP 协议类型
/// 
/// 直接复用 server 模块的协议检测逻辑
pub async fn detect_mcp_protocol(url: &str) -> Result<McpProtocol> {
    crate::server::detect_mcp_protocol(url).await
}
