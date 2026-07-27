# MCP Streamable HTTP Proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP Streamable HTTP Proxy

基于 rmcp 0.12 的 MCP Streamable HTTP 代理实现，支持有状态会话管理。

## 概述

此模块为 MCP (Model Context Protocol) 使用 Streamable HTTP 传输提供代理实现，并具备有状态会话管理功能。

## 功能特性

- **Streamable HTTP 支持**: 使用 rmcp Streamable HTTP 传输
- **有状态会话**: 自定义 SessionManager
- **后端隔离**:
  - **URL**：默认 **每 session 独立**后端（`initialize` 时建连，Drop/RAII 释放）
  - **Stdio**：默认 **共享**单子进程 + 通知 fan-out
- **热交换**: 共享隔离模式下支持后端热替换
- **版本控制**: 共享后端重连时使旧 session 失效

## 架构

```text
客户端 → Streamable HTTP → ProxyAwareSessionManager → ProxyHandler → 后端 MCP 服务
  URL（per-session）：factory 新建 handler → initialize 建连 → Drop 断连
  Stdio（shared）：单一后端 Arc + UpstreamPeerRegistry
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
