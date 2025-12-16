# mcp-stdio-proxy

MCP (Model Context Protocol) 代理服务器和 CLI 工具，支持协议转换和远程服务访问。将远程 MCP 服务（SSE/HTTP）转换为本地 stdio 接口。

> **注意**：虽然包名是 `mcp-stdio-proxy`，但 CLI 命令名是 `mcp-proxy`（更简短）。

## 功能特性

- 🔄 **协议转换**：支持 SSE、Streamable HTTP 和 Stdio 协议之间的转换
- 🌐 **远程代理**：将远程 MCP 服务转换为本地 stdio 接口
- 🔍 **自动协议检测**：智能识别 MCP 服务的协议类型
- 🚀 **CLI 工具**：提供命令行工具，方便集成到各种工作流
- 📡 **多种传输方式**：支持 Server-Sent Events (SSE) 和 Streamable HTTP

## 安装

### 从 crates.io 安装

```bash
cargo install mcp-stdio-proxy

# 安装后，使用 mcp-proxy 命令
mcp-proxy --help
```

### 从源码构建

```bash
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy/mcp-proxy
cargo build --release
```

## CLI 使用

### 协议转换模式

将远程 MCP 服务（SSE 或 Streamable HTTP）转换为本地 stdio 接口：

```bash
# 基本用法
mcp-proxy convert https://example.com/mcp/sse

# 带认证
mcp-proxy convert https://example.com/mcp/sse --auth "Bearer your-token"

# 自定义 HTTP headers
mcp-proxy convert https://example.com/mcp/sse \
  --header "X-Custom-Header=value" \
  --header "Authorization=Bearer token"

# 设置超时和重试
mcp-proxy convert https://example.com/mcp/sse \
  --timeout 60 \
  --retries 5
```

### 服务状态检查

检查 MCP 服务是否可用：

```bash
mcp-proxy check https://example.com/mcp/sse
mcp-proxy check https://example.com/mcp/sse --auth "Bearer token" --timeout 10
```

### 协议检测

自动检测 MCP 服务的协议类型：

```bash
mcp-proxy detect https://example.com/mcp/sse
mcp-proxy detect https://example.com/mcp/sse --auth "Bearer token"
```

### 向后兼容模式

直接使用 URL 作为参数（向后兼容）：

```bash
mcp-proxy https://example.com/mcp/sse
```

## 使用场景

### 场景 1：将远程 SSE 服务转换为 stdio

```bash
# 启动代理转换
mcp-proxy convert https://remote-server.com/mcp/sse --auth "Bearer token" | \
  your-mcp-client
```

### 场景 2：在 CI/CD 中使用

```bash
# 检查服务状态
if mcp-proxy check https://api.example.com/mcp; then
  echo "服务正常"
else
  echo "服务不可用"
  exit 1
fi
```

### 场景 3：协议检测脚本

```bash
# 检测协议类型
PROTOCOL=$(mcp-proxy detect https://example.com/mcp --quiet)
echo "检测到协议: $PROTOCOL"
```

## 支持的协议

| 协议类型 | 说明 | 支持状态 |
|---------|------|---------|
| SSE | Server-Sent Events，单向实时通信 | ✅ |
| Streamable HTTP | 高效的双向通信协议 | ✅ |
| Stdio | 标准输入输出，用于命令行启动的服务 | ✅ (仅作为输出) |

## 命令行选项

### 全局选项

- `-v, --verbose`: 详细输出模式
- `-q, --quiet`: 静默模式，只输出必要信息

### Convert 命令选项

- `--auth <AUTH>`: 认证 header（如: "Bearer token"）
- `-H, --header <KEY=VALUE>`: 自定义 HTTP headers（可多次使用）
- `--timeout <SECONDS>`: 连接超时时间（默认: 30 秒）
- `--retries <NUM>`: 重试次数（默认: 3 次）

### Check 命令选项

- `--auth <AUTH>`: 认证 header
- `--timeout <SECONDS>`: 超时时间（默认: 10 秒）

## 示例

### 完整示例：连接远程 MCP 服务

```bash
# 1. 检测协议
mcp-proxy detect https://api.example.com/mcp

# 2. 检查服务状态
mcp-proxy check https://api.example.com/mcp --auth "Bearer your-token"

# 3. 转换为 stdio 并连接到客户端
mcp-proxy convert https://api.example.com/mcp \
  --auth "Bearer your-token" \
  --timeout 60 | \
  your-mcp-client
```

## 服务器模式

除了 CLI 模式，`mcp-proxy` 还支持作为 HTTP 服务器运行：

```bash
# 启动服务器（默认端口 8080）
mcp-proxy

# 查看帮助
mcp-proxy --help
```

服务器模式提供完整的 MCP 代理功能，支持动态添加和管理 MCP 服务。

## 依赖要求

- Rust 1.70+ (推荐 1.75+)
- 网络连接（用于访问远程 MCP 服务）

## 许可证

本项目采用 MIT 或 Apache-2.0 双许可证。

## 贡献

欢迎提交 Issue 和 Pull Request！

GitHub 仓库: https://github.com/nuwax-ai/mcp-proxy

## 相关项目

- [rmcp](https://crates.io/crates/rmcp) - Rust MCP 协议实现库

