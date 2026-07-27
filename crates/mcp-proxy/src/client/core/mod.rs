//! 核心业务逻辑模块
//!
//! 包含协议转换的核心实现，与 CLI 接口解耦

pub mod command;
pub mod common;
pub mod convert;
pub mod sse;
pub mod stream;

// 导出公共 API
// 注意: run_sse_mode 和 run_stream_mode 是内部实现细节，
// 只被 convert 模块使用，不需要对外暴露
pub use command::run_command_mode;
pub use convert::run_url_mode_with_retry;
