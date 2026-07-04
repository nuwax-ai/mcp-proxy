# MCP Streamable HTTP Proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP Streamable HTTP Proxy

基于 rmcp 0.12 的 MCP Streamable HTTP 代理实现，支持有状态会话管理。

## 概述

此模块为 MCP (Model Context Protocol) 使用 Streamable HTTP 传输提供代理实现，并具备有状态会话管理功能。

## 功能特性

- **Streamable HTTP 支持**: 使用 rmcp 0.12 增强的 Streamable HTTP 传输
- **有状态会话**: 使用 DashMap 进行后端版本跟踪的自定义 SessionManager
- **热交换**: 支持无停机后端连接替换
- **版本控制**: 后端重连时自动使会话失效
- **高级客户端 API**: 简单的连接接口，隐藏传输细节

## 架构

```text
客户端 → Streamable HTTP → ProxyAwareSessionManager → ProxyHandler → 后端 MCP 服务
                                      ↓
                              版本跟踪
                              (DashMap<SessionId, BackendVersion>)
```

## 安装

添加到 `Cargo.toml`:

```toml
[dependencies]
mcp-streamable-proxy = { version = "0.1.5", path = "../mcp-streamable-proxy" }
```

## 使用

### 服务端

```rust
use mcp_streamable_proxy::{McpServiceConfig, run_stream_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = McpServiceConfig::new("my-service".to_string());

    run_stream_server(config).await?;

    Ok(())
}
```

### 客户端

```rust
use mcp_streamable_proxy::{StreamClientConnection, McpClientConfig};

// 连接到 MCP 服务器
let config = McpClientConfig::new("http://localhost:8080/mcp");
let conn = StreamClientConnection::connect(config).await?;

// 列出可用工具
let tools = conn.list_tools().await?;
```

## 会话管理

`ProxyAwareSessionManager` 提供：
- 使用 DashMap 进行后端版本跟踪
- 后端重连时自动会话失效
- 并发安全的会话操作

## 开发

```bash
# 构建
cargo build -p mcp-streamable-proxy

# 测试
cargo test -p mcp-streamable-proxy
```

## 许可证

MIT OR Apache-2.0
