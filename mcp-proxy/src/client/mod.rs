// MCP 客户端模块
// 提供各种 MCP 协议的客户端实现和 CLI 工具

mod cli;
mod protocol;
pub(crate) mod proxy_server;
pub mod test_mcp_server;

#[cfg(test)]
mod tests;

// 导出 CLI 功能 - 使用真实实现
pub use cli::{Cli, Commands, run_cli};

