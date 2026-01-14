# MCP Common

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP Common

MCP 代理组件的共享类型和工具。

## 概述

`mcp-common` 为 `mcp-sse-proxy` 和 `mcp-streamable-proxy` 提供共享功能，避免代码重复。

## 功能特性

- **配置类型**: `McpServiceConfig`、`McpClientConfig` 用于统一配置管理
- **工具过滤**: `ToolFilter` 用于按名称或模式过滤 MCP 工具
- **OpenTelemetry 支持**: 可选的遥测功能用于分布式追踪

## 功能标志

- `telemetry`: 基础 OpenTelemetry 支持
- `otlp`: OTLP 导出器支持（用于 Jaeger 等）

## 安装

添加到 `Cargo.toml`:

```toml
[dependencies]
mcp-common = { version = "0.1.5", path = "../mcp-common" }
```

## 使用

```rust
use mcp_common::{McpServiceConfig, McpClientConfig, ToolFilter};

// 创建客户端配置
let client_config = McpClientConfig::new("http://localhost:8080/mcp")
    .with_headers(vec![("Authorization".to_string(), "Bearer token".to_string())]);

// 创建服务配置
let service_config = McpServiceConfig::new("my-service".to_string())
    .with_persistent_type();

// 使用工具过滤器
let filter = ToolFilter::new(vec!["tool1".to_string(), "tool2".to_string()]);
```

## 开发

```bash
# 构建
cargo build -p mcp-common

# 测试
cargo test -p mcp-common

# 启用功能
cargo build -p mcp-common --features telemetry,otlp
```

## 许可证

MIT OR Apache-2.0
