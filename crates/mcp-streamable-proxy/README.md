# MCP Streamable HTTP Proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP Streamable HTTP Proxy

Streamable HTTP proxy implementation for MCP using rmcp 0.12 with stateful session management.

## Overview

This module provides a proxy implementation for MCP (Model Context Protocol) using Streamable HTTP transport with stateful session management.

## Features

- **Streamable HTTP Support**: Uses rmcp with Streamable HTTP transport
- **Stateful Sessions**: Custom SessionManager with backend version tracking
- **Backend isolation**:
  - **URL backends** default to **per-session** connections (connect on `initialize`, RAII drop)
  - **Stdio backends** default to **shared** single child process + notification fan-out
- **Hot Swap**: Supports backend connection replacement for shared isolation
- **Version Control**: Invalidates sessions when a shared backend reconnects
- **High-level Client API**: Simple connection interface hiding transport details

## Architecture

```text
Client → Streamable HTTP → ProxyAwareSessionManager → ProxyHandler → Backend MCP Service

URL (per-session):
  each factory() → new ProxyHandler → initialize → connect() → Drop closes backend

Stdio (shared):
  one backend Arc shared by cloned handlers + UpstreamPeerRegistry fan-out
```

### Builder knobs

```rust,ignore
StreamServerBuilder::new(BackendConfig::Url { url, headers: None })
    .stateful(true)
    .backend_isolation(BackendIsolation::PerSession) // default for URL
    .max_sessions(32)
    .connect_timeout(Duration::from_secs(30))
    .build()
    .await?;
```

Stdio + `PerSession` is rejected in this release.
## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
mcp-streamable-proxy = { version = "0.1.5", path = "../mcp-streamable-proxy" }
```

## Usage

### Server

```rust
use mcp_streamable_proxy::{McpServiceConfig, run_stream_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = McpServiceConfig::new("my-service".to_string());

    run_stream_server(config).await?;

    Ok(())
}
```

### Client

```rust
use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};

// Connect to an MCP server
let config = McpClientConfig::new("http://localhost:8080/mcp");
let conn = StreamClientConnection::connect(config).await?;

// List available tools
let tools = conn.list_tools().await?;
```

## Session Management

The `ProxyAwareSessionManager` provides:
- Backend version tracking using DashMap
- Automatic session invalidation on backend reconnect
- Concurrent-safe session operations

## Development

```bash
# Build
cargo build -p mcp-streamable-proxy

# Test
cargo test -p mcp-streamable-proxy
```

## License

MIT OR Apache-2.0
