//! 支持功能模块
//!
//! 提供配置、日志、工具函数等基础设施支持

pub mod args;
pub mod config;
pub mod diagnostic;
pub mod logging;
pub mod utils;

#[cfg(test)]
mod config_tests;

// 导出常用类型
pub use args::{CheckArgs, ConvertArgs, DetectArgs, LoggingArgs, parse_key_val};
pub use config::{McpConfigSource, merge_headers, parse_convert_config};
pub use diagnostic::{classify_error, print_diagnostic_report, summarize_error};
pub use logging::{generate_session_id, init_logging, init_logging_with_config};
pub use utils::{protocol_name, truncate_str};
