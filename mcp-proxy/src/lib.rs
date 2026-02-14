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
pub use model::{AppState, DynamicRouterService, ProxyHandlerManager, get_proxy_manager};
pub use proxy::{McpHandler, ProxyHandler, StreamProxyHandler};
pub use proxy::{SseBackendConfig, SseServerBuilder, StreamBackendConfig, StreamServerBuilder};
pub use server::{
    create_telemetry_layer, get_health, get_ready, get_router, init_tracer_provider,
    log_service_info, mcp_start_task, schedule_check_mcp_live, set_layer, shutdown_telemetry,
    start_schedule_task,
};

// 导出 CLI 功能
pub use client::{Cli, Commands, run_cli};

// 导出用于基准测试的组件
pub use server::handlers::run_code_handler::{RunCodeMessageRequest, run_code_handler};
