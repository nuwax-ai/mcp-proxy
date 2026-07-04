# MCP SSE Proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP SSE Proxy

SSE (Server-Sent Events) proxy implementation for MCP using rmcp 0.10.

## Overview

This module provides a proxy implementation for MCP (Model Context Protocol) using SSE (Server-Sent Events) transport.

## Features

- **SSE Support**: Uses rmcp 0.10 with SSE transport (removed in 0.12+)
- **Stable Protocol**: Production-ready SSE implementation
- **Hot Swap**: Supports backend connection replacement
- **High-level Client API**: Simple connection interface hiding transport details

## Architecture

```text
Client → SSE → SseHandler → Backend MCP Service
```

## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
mcp-sse-proxy = { version = "0.1.5", path = "../mcp-sse-proxy" }
```

## Usage

### Server

```rust
use mcp_sse_proxy::{McpServiceConfig, run_sse_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = McpServiceConfig::new("my-service".to_string());

    run_sse_server(config).await?;

    Ok(())
}
```

### Client

```rust
use mcp_sse_proxy::{SseClientConnection, McpClientConfig};

// Connect to an MCP server
let config = McpClientConfig::new("http://localhost:8080/sse");
let conn = SseClientConnection::connect(config).await?;

// List available tools
let tools = conn.list_tools().await?;
```

## Development

```bash
# Build
cargo build -p mcp-sse-proxy

# Test
cargo test -p mcp-sse-proxy
```

## License

MIT OR Apache-2.0
