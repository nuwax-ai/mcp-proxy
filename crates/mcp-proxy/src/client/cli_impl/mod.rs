//! CLI 实现模块
//!
//! 处理 CLI 命令的实现和参数解析

pub mod check;
pub mod convert_cmd;
pub mod health;

// 导出 CLI 命令实现
pub use check::{run_check_command, run_detect_command};
pub use convert_cmd::run_convert_command;
pub use health::run_health_command;
