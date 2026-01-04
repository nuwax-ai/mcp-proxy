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
pub mod tool_filter;

#[cfg(feature = "telemetry")]
pub mod telemetry;

// Re-export main types
pub use client_config::McpClientConfig;
pub use config::McpServiceConfig;
pub use tool_filter::ToolFilter;

// Re-export telemetry types when feature is enabled
#[cfg(feature = "telemetry")]
pub use telemetry::{create_otel_layer, init_tracing, TracingConfig, TracingGuard};
