//! MCP Common - Shared types and utilities for MCP proxy modules
//!
//! This crate provides common functionality shared across mcp-sse-proxy
//! and mcp-streamable-proxy to avoid code duplication.
//!
//! # Feature Flags
//!
//! - `telemetry`: 基础 OpenTelemetry 支持
//! - `otlp`: OTLP exporter 支持（用于 Jaeger 等）
//!
//! # 国际化 (i18n)
//!
//! 本 crate 提供多语言支持，使用 rust-i18n 实现。
//!
//! ## 使用方法
//!
//! ```rust
//! use mcp_common::{t, set_locale, init_locale_from_env};
//!
//! // 初始化语言设置（程序启动时调用）
//! init_locale_from_env();
//!
//! // 获取翻译
//! let msg = t!("errors.mcp_proxy.service_not_found", service = "my-service");
//! ```

// 初始化 i18n，必须在 crate root 调用
#[macro_use]
extern crate rust_i18n;

// 初始化翻译文件，使用 crate 内置 locales（支持独立发布）
i18n!("locales", fallback = "en");

pub mod backend_bridge;
pub mod client_config;
pub mod config;
pub mod diagnostic;
pub mod i18n;
pub mod mirror;
pub mod process_compat;
pub mod tool_filter;

#[cfg(feature = "telemetry")]
pub mod telemetry;

// Re-export main types
pub use backend_bridge::BackendBridge;
pub use client_config::McpClientConfig;
pub use config::McpServiceConfig;
pub use process_compat::check_windows_command;
pub use process_compat::ensure_runtime_path;
pub use process_compat::resolve_windows_command;
pub use process_compat::spawn_stderr_reader;
pub use tool_filter::ToolFilter;

// Re-export i18n types
pub use i18n::{
    AVAILABLE_LOCALES, DEFAULT_LOCALE, current_locale, init_locale_from_env, set_locale, t,
};

// Re-export telemetry types when feature is enabled
#[cfg(feature = "telemetry")]
pub use telemetry::{TracingConfig, TracingGuard, create_otel_layer, init_tracing};
