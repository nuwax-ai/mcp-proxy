# MCP Common

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP Common

Shared types and utilities for MCP proxy components.

## Overview

`mcp-common` provides common functionality shared across `mcp-sse-proxy` and `mcp-streamable-proxy` to avoid code duplication.

## Features

- **Configuration Types**: `McpServiceConfig`, `McpClientConfig` for unified configuration management
- **Tool Filtering**: `ToolFilter` for filtering MCP tools by name or pattern
- **OpenTelemetry Support**: Optional telemetry features for distributed tracing

## Feature Flags

- `telemetry`: Basic OpenTelemetry support
- `otlp`: OTLP exporter support (for Jaeger, etc.)

## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
mcp-common = { version = "0.1.5", path = "../mcp-common" }
```

## Usage

```rust
use mcp_common::{McpServiceConfig, McpClientConfig, ToolFilter};

// Create client configuration
let client_config = McpClientConfig::new("http://localhost:8080/mcp")
    .with_headers(vec![("Authorization".to_string(), "Bearer token".to_string())]);

// Create service configuration
let service_config = McpServiceConfig::new("my-service".to_string())
    .with_persistent_type();

// Use tool filter
let filter = ToolFilter::new(vec!["tool1".to_string(), "tool2".to_string()]);
```

## Development

```bash
# Build
cargo build -p mcp-common

# Test
cargo test -p mcp-common

# With features
cargo build -p mcp-common --features telemetry,otlp
```

## License

MIT OR Apache-2.0
