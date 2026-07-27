// 初始化 i18n，使用 crate 内置翻译文件
#[macro_use]
extern crate rust_i18n;

// 初始化翻译文件，使用 crate 内置 locales（支持独立发布）
i18n!("locales", fallback = "en");

mod client;
mod config;
pub mod env_init;
mod mcp_error;
mod model;
mod proxy;
mod server;
#[cfg(test)]
mod tests;

// 导出基础功能
pub use config::AppConfig;
pub use mcp_error::AppError;
pub use model::{
    AppState, DynamicRouterService, McpConfig, McpProtocol, McpType, ProxyHandlerManager,
    get_proxy_manager,
};
pub use proxy::{McpHandler, ProxyHandler, StreamProxyHandler};
pub use proxy::{SseBackendConfig, SseServerBuilder, StreamBackendConfig, StreamServerBuilder};
pub use server::{
    create_telemetry_layer, get_health, get_ready, get_router, init_tracer_provider,
    log_service_info, mcp_start_task, schedule_check_mcp_live, set_layer, shutdown_telemetry,
    start_schedule_task,
};

// 导出 CLI 功能
pub use client::{Cli, Commands, run_cli};

// 导出 i18n 功能
pub use mcp_common::{current_locale, init_locale_from_env, set_locale, t};

// 导出用于基准测试的组件
pub use server::handlers::run_code_handler::{RunCodeMessageRequest, run_code_handler};
