# MCP SSE Proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP SSE Proxy

基于 rmcp 0.10 的 MCP SSE (Server-Sent Events) 代理实现。

## 概述

此模块为 MCP (Model Context Protocol) 使用 SSE (Server-Sent Events) 传输提供代理实现。

## 功能特性

- **SSE 支持**: 使用 rmcp 0.10 的 SSE 传输（在 0.12+ 版本中已移除）
- **稳定协议**: 生产就绪的 SSE 实现
- **热交换**: 支持后端连接替换
- **高级客户端 API**: 简单的连接接口，隐藏传输细节

## 架构

```text
客户端 → SSE → SseHandler → 后端 MCP 服务
```

## 安装

添加到 `Cargo.toml`:

```toml
[dependencies]
mcp-sse-proxy = { version = "0.1.5", path = "../mcp-sse-proxy" }
```

## 使用

### 服务端

```rust
use mcp_sse_proxy::{McpServiceConfig, run_sse_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = McpServiceConfig::new("my-service".to_string());

    run_sse_server(config).await?;

    Ok(())
}
```

### 客户端

```rust
use mcp_sse_proxy::{SseClientConnection, McpClientConfig};

// 连接到 MCP 服务器
let config = McpClientConfig::new("http://localhost:8080/sse");
let conn = SseClientConnection::connect(config).await?;

// 列出可用工具
let tools = conn.list_tools().await?;
```

## 开发

```bash
# 构建
cargo build -p mcp-sse-proxy

# 测试
cargo test -p mcp-sse-proxy
```

## 许可证

MIT OR Apache-2.0
