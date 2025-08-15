pub mod config;
pub mod error;
pub mod models;
pub mod app_state;
pub mod parsers;
pub mod processors;
pub mod services;
pub mod handlers;
pub mod routes;
pub mod utils;
pub mod middleware;
pub mod performance;
pub mod production;

#[cfg(test)]
mod tests;

pub use config::AppConfig;
pub use error::AppError;
pub use app_state::AppState;
pub use models::*;

/// 应用版本
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 应用名称
pub const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// 应用描述
pub const APP_DESCRIPTION: &str = "多格式文档解析服务 - 支持PDF、Word、Excel、PowerPoint等格式转换为结构化Markdown";

/// 默认配置
pub fn get_default_config() -> AppConfig {
    serde_yaml::from_str(include_str!("../config.yml"))
        .expect("默认配置应该有效")
}
