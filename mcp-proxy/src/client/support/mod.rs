//! 支持功能模块
//!
//! 提供配置、日志、工具函数等基础设施支持

pub mod args;
pub mod config;
pub mod logging;
pub mod utils;
pub mod diagnostic;

#[cfg(test)]
mod config_tests;

// 导出常用类型
pub use args::{ConvertArgs, CheckArgs, DetectArgs, parse_key_val, LoggingArgs};
pub use config::{McpConfigSource, parse_convert_config, merge_headers};
pub use logging::{init_logging, init_logging_with_config, generate_session_id};
pub use utils::{protocol_name, truncate_str};
pub use diagnostic::{classify_error, summarize_error, print_diagnostic_report};
