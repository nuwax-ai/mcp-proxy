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
pub use args::{CheckArgs, ConvertArgs, DetectArgs, HealthArgs, LoggingArgs};
pub use config::{McpConfigSource, merge_headers, parse_convert_config};
pub(crate) use config::{merge_config_headers_checked, merge_headers_checked};
pub use diagnostic::{classify_error, print_diagnostic_report, summarize_error};
pub use logging::{init_logging, init_logging_with_config};
pub use utils::protocol_name;
