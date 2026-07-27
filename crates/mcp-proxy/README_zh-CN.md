# mcp-stdio-proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# mcp-stdio-proxy

MCP (Model Context Protocol) 客户端代理工具，将远程 MCP 服务（SSE/Streamable HTTP）转换为本地 stdio 接口。

> **包名**：`mcp-stdio-proxy`
> **命令名**：`mcp-proxy`（更简短）

## 核心功能

`mcp-proxy` 是一个轻量级的客户端代理工具，解决了一个核心问题：

**让只支持 stdio 协议的 MCP 客户端能够访问远程的 SSE/HTTP MCP 服务。**

### 工作原理

```
远程 MCP 服务 (SSE/HTTP) ←→ mcp-proxy ←→ 本地应用 (stdio)
```

- **输入**：远程 MCP 服务 URL（支持 SSE 或 Streamable HTTP 协议）
- **输出**：本地 stdio 接口（标准输入/输出）
- **作用**：协议转换 + 透明代理

## 功能特性

- 🔄 **协议转换**：自动检测并转换 SSE/Streamable HTTP → stdio
- 🌐 **远程访问**：让本地应用能够访问远程 MCP 服务
- 🔍 **自动协议检测**：智能识别服务端协议类型
- 🔐 **认证支持**：支持自定义 Authorization header 和其他 HTTP headers
- ⚡ **轻量高效**：无需额外配置，开箱即用

## 安装

### 从 crates.io 安装（推荐）

```bash
cargo install mcp-stdio-proxy
```

### 从源码构建

```bash
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy/mcp-proxy
cargo build --release
# 二进制文件位于: target/release/mcp-proxy
```

## 快速开始

### 基本用法

```bash
# 将远程 SSE 服务转换为 stdio
mcp-proxy convert https://example.com/mcp/sse

# 或使用简化语法（向后兼容）
mcp-proxy https://example.com/mcp/sse
```

### 带认证的完整示例

```bash
# 使用 Bearer token 认证
mcp-proxy convert https://api.example.com/mcp/sse \
  --auth "Bearer your-api-token"

# 添加自定义 headers
mcp-proxy convert https://api.example.com/mcp/sse \
  -H "Authorization=Bearer token" \
  -H "X-Custom-Header=value"
```

### 配合 MCP 客户端使用

```bash
# 将 mcp-proxy 输出管道到你的 MCP 客户端
mcp-proxy convert https://remote-server.com/mcp \
  --auth "Bearer token" | \
  your-mcp-client

# 或在 MCP 客户端配置中使用
# 配置文件示例（如 Claude Desktop 配置）：
{
  "mcpServers": {
    "remote-service": {
      "command": "mcp-proxy",
      "args": [
        "convert",
        "https://remote-server.com/mcp/sse",
        "--auth",
        "Bearer your-token"
      ]
    }
  }
}
```

## 命令详解

### 1. `convert` - 协议转换（核心命令）

将远程 MCP 服务转换为本地 stdio 接口。

```bash
mcp-proxy convert <URL> [选项]
```

**选项：**

| 选项 | 简写 | 说明 | 默认值 |
|------|------|------|--------|
| `--auth <TOKEN>` | `-a` | 认证 header（如: "Bearer token"） | - |
| `--header <KEY=VALUE>` | `-H` | 自定义 HTTP headers（可多次使用） | - |
| `--timeout <SECONDS>` | - | 连接超时时间（秒） | 30 |
| `--retries <NUM>` | - | 重试次数 | 3 |
| `--verbose` | `-v` | 详细输出（显示调试信息） | false |
| `--quiet` | `-q` | 静默模式（只输出错误） | false |

**示例：**

```bash
# 基本转换
mcp-proxy convert https://api.example.com/mcp/sse

# 带认证和超时设置
mcp-proxy convert https://api.example.com/mcp/sse \
  --auth "Bearer sk-1234567890" \
  --timeout 60 \
  --retries 5

# 添加多个自定义 headers
mcp-proxy convert https://api.example.com/mcp \
  -H "Authorization=Bearer token" \
  -H "X-API-Key=your-key" \
  -H "X-Request-ID=abc123"

# 详细模式（查看连接过程）
mcp-proxy convert https://api.example.com/mcp/sse --verbose
```

### 2. `check` - 服务状态检查

检查远程 MCP 服务是否可用，验证连接性和协议支持。

```bash
mcp-proxy check <URL> [选项]
```

**选项：**

| 选项 | 简写 | 说明 | 默认值 |
|------|------|------|--------|
| `--auth <TOKEN>` | `-a` | 认证 header | - |
| `--timeout <SECONDS>` | - | 超时时间（秒） | 10 |

**示例：**

```bash
# 检查服务状态
mcp-proxy check https://api.example.com/mcp/sse

# 带认证检查
mcp-proxy check https://api.example.com/mcp/sse \
  --auth "Bearer token" \
  --timeout 5
```

**退出码：**
- `0`：服务正常
- `非 0`：服务不可用或检查失败

### 3. `detect` - 协议检测

自动检测远程 MCP 服务使用的协议类型。

```bash
mcp-proxy detect <URL> [选项]
```

**选项：**

| 选项 | 简写 | 说明 |
|------|------|------|
| `--auth <TOKEN>` | `-a` | 认证 header |
| `--quiet` | `-q` | 静默模式（只输出协议类型） |

**输出：**
- `SSE` - Server-Sent Events 协议
- `Streamable HTTP` - Streamable HTTP 协议
- `Stdio` - 标准输入输出协议（不适用于远程服务）

**示例：**

```bash
# 检测协议类型
mcp-proxy detect https://api.example.com/mcp/sse

# 在脚本中使用
PROTOCOL=$(mcp-proxy detect https://api.example.com/mcp --quiet)
if [ "$PROTOCOL" = "SSE" ]; then
  echo "检测到 SSE 协议"
fi
```

## 使用场景

### 场景 1：Claude Desktop 集成远程 MCP 服务

Claude Desktop 只支持 stdio 协议的 MCP 服务，使用 `mcp-proxy` 可以让它访问远程服务。

**配置文件示例** (`~/Library/Application Support/Claude/config.json`)：

```json
{
  "mcpServers": {
    "remote-database": {
      "command": "mcp-proxy",
      "args": [
        "convert",
        "https://your-server.com/mcp/database",
        "--auth",
        "Bearer your-token-here"
      ]
    },
    "remote-search": {
      "command": "mcp-proxy",
      "args": ["https://search-api.com/mcp/sse"]
    }
  }
}
```

### 场景 2：CI/CD 流水线中的健康检查

```bash
#!/bin/bash
# 部署前检查 MCP 服务状态

echo "检查 MCP 服务..."
if mcp-proxy check https://api.example.com/mcp --timeout 5; then
  echo "✅ MCP 服务正常，继续部署"
  # 执行部署脚本
  ./deploy.sh
else
  echo "❌ MCP 服务不可用，中止部署"
  exit 1
fi
```

### 场景 3：跨网络访问企业内部 MCP 服务

```bash
# 通过 VPN 或跳板机访问内网 MCP 服务
mcp-proxy convert https://internal-mcp.company.com/api/sse \
  --auth "Bearer ${MCP_TOKEN}" \
  --timeout 120 | \
  local-mcp-client
```

### 场景 4：开发测试

```bash
# 快速测试远程 MCP 服务
mcp-proxy convert https://test-api.com/mcp/sse --verbose

# 查看详细的连接和通信日志
RUST_LOG=debug mcp-proxy convert https://api.com/mcp/sse -v
```

## 支持的协议

`mcp-proxy` 可以连接以下协议的远程 MCP 服务：

| 协议 | 说明 | 状态 |
|------|------|------|
| **SSE** | Server-Sent Events，单向实时推送 | ✅ 完全支持 |
| **Streamable HTTP** | 双向流式 HTTP 通信 | ✅ 完全支持 |

**输出协议**：始终是 **stdio**（标准输入/输出）

## 环境变量

| 变量 | 说明 | 示例 |
|------|------|------|
| `RUST_LOG` | 日志级别 | `RUST_LOG=debug mcp-proxy convert ...` |
| `HTTP_PROXY` | HTTP 代理 | `HTTP_PROXY=http://proxy:8080` |
| `HTTPS_PROXY` | HTTPS 代理 | `HTTPS_PROXY=http://proxy:8080` |

## 常见问题

### Q: 为什么需要 mcp-proxy？

**A:** 许多 MCP 客户端（如 Claude Desktop）只支持本地 stdio 协议的服务。如果你的 MCP 服务部署在远程服务器上使用 SSE 或 HTTP 协议，就需要 `mcp-proxy` 作为协议转换桥梁。

### Q: mcp-proxy 和 MCP 服务器有什么区别？

**A:**
- **MCP 服务器**：提供具体功能（数据库访问、文件操作等）的后端服务
- **mcp-proxy**：纯粹的客户端代理工具，只做协议转换，不提供任何业务功能

### Q: 支持双向通信吗？

**A:** 是的！无论是 SSE 还是 Streamable HTTP 协议，`mcp-proxy` 都支持完整的双向通信（请求/响应）。

### Q: 如何调试连接问题？

**A:** 使用 `--verbose` 选项和 `RUST_LOG` 环境变量：

```bash
RUST_LOG=debug mcp-proxy convert https://api.com/mcp --verbose
```

### Q: 支持自签名 SSL 证书吗？

**A:** 当前版本使用系统默认的证书验证。如需支持自签名证书，请提交 Issue。

## 故障排除

### 连接超时

```bash
# 增加超时时间
mcp-proxy convert https://slow-api.com/mcp --timeout 120
```

### 认证失败

```bash
# 检查 token 格式，确保包含 "Bearer " 前缀
mcp-proxy convert https://api.com/mcp --auth "Bearer your-token-here"

# 或使用自定义 header
mcp-proxy convert https://api.com/mcp -H "Authorization=Bearer your-token"
```

### 协议检测失败

```bash
# 查看详细错误信息
mcp-proxy detect https://api.com/mcp --verbose

# 检查服务状态
mcp-proxy check https://api.com/mcp
```

## 系统要求

- **操作系统**：Linux, macOS, Windows
- **Rust 版本**：1.70+ （仅从源码构建时需要）
- **网络**：能够访问目标 MCP 服务

## 许可证

本项目采用 MIT 或 Apache-2.0 双许可证。

## 贡献

欢迎提交 Issue 和 Pull Request！

- **GitHub 仓库**：https://github.com/nuwax-ai/mcp-proxy
- **问题反馈**：https://github.com/nuwax-ai/mcp-proxy/issues
- **功能建议**：https://github.com/nuwax-ai/mcp-proxy/discussions

## 相关资源

- [MCP 官方文档](https://modelcontextprotocol.io/)
- [rmcp - Rust MCP 实现](https://crates.io/crates/rmcp)
- [MCP 服务器列表](https://github.com/modelcontextprotocol/servers)

## 更新日志

### v0.1.18

- ✅ 支持 SSE 和 Streamable HTTP 协议转换
- ✅ 自动协议检测
- ✅ 认证和自定义 headers 支持
- ✅ 服务状态检查命令
- ✅ 协议检测命令
- ✅ OpenTelemetry 集成，支持 OTLP
- ✅ 后台健康检查
- ✅ 通过外部进程执行代码
