//! MCP Common - Shared types and utilities for MCP proxy modules
//!
//! This crate provides common functionality shared across mcp-sse-proxy
//! and mcp-streamable-proxy to avoid code duplication.

pub mod tool_filter;
pub mod config;

// Re-export main types
pub use tool_filter::ToolFilter;
pub use config::McpServiceConfig;
