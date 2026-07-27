// MCP 客户端模块
// 提供各种 MCP 协议的客户端实现和 CLI 工具

mod cli;
mod protocol;
pub(crate) mod proxy_server;

// 新的模块化架构 (按功能层次分组)
pub mod cli_impl; // CLI 命令实现
pub mod core; // 核心业务逻辑
pub mod support; // 支持功能

#[cfg(test)]
mod tests;

// 导出 CLI 功能（公共 API）
pub use cli::{Cli, Commands, run_cli};

// 注意：ConvertArgs, CheckArgs, DetectArgs 等类型只在内部使用，
// 不需要在这里重新导出。如需使用，请通过 support 模块导入。
