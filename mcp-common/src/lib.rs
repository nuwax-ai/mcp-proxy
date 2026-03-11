//! MCP Common - Shared types and utilities for MCP proxy modules
//!
//! This crate provides common functionality shared across mcp-sse-proxy
//! and mcp-streamable-proxy to avoid code duplication.
//!
//! # Feature Flags
//!
//! - `telemetry`: 基础 OpenTelemetry 支持
//! - `otlp`: OTLP exporter 支持（用于 Jaeger 等）

pub mod client_config;
pub mod config;
pub mod diagnostic;
pub mod mirror;
pub mod process_compat;
pub mod tool_filter;

#[cfg(feature = "telemetry")]
pub mod telemetry;

// Re-export main types
pub use client_config::McpClientConfig;
pub use config::McpServiceConfig;
pub use process_compat::check_windows_command;
pub use process_compat::convert_path_to_windows_format;
pub use process_compat::ensure_runtime_path;
pub use process_compat::prepare_stdio_env;
pub use process_compat::preprocess_npx_command_windows;
pub use process_compat::resolve_windows_command;
pub use process_compat::spawn_stderr_reader;
pub use tool_filter::ToolFilter;

// Re-export telemetry types when feature is enabled
#[cfg(feature = "telemetry")]
pub use telemetry::{TracingConfig, TracingGuard, create_otel_layer, init_tracing};
