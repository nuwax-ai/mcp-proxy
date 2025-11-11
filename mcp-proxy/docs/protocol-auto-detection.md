# MCP 协议自动检测功能

## 概述

MCP 代理服务现在支持自动检测后端 MCP 服务的协议类型，无需手动指定 `backendProtocol` 参数。

## 功能特性

### 1. 自动协议检测

当你不指定 `backendProtocol` 参数时，系统会自动检测远程 MCP 服务使用的协议：

- **Streamable HTTP 协议检测**：
  - 发送带有 `Accept: application/json, text/event-stream` 头的 POST 请求
  - 检查响应头中的 `mcp-session-id`（Streamable HTTP 的特征）
  - 检查 `Content-Type` 是否为 `text/event-stream` 或 `application/json`
  - 检查是否返回 `406 Not Acceptable`（说明需要特定的 Accept 头）

- **SSE 协议检测**：
  - 发送 GET 请求到服务端点
  - 检查响应头中的 `Content-Type: text/event-stream`

### 2. 协议转换

支持客户端协议和后端协议的独立配置：

- **客户端协议**：由请求路径决定（`/mcp/sse/` 或 `/mcp/stream/`）
- **后端协议**：自动检测或手动指定

这样可以实现：
- SSE 客户端 ↔ Streamable HTTP 后端
- Streamable HTTP 客户端 ↔ SSE 后端
- 同协议透明代理

## 使用方法

### 方式一：自动检测（推荐）

```http
POST http://localhost:8085/mcp/sse/check_status
Content-Type: application/json

{
  "mcpId": "my-service",
  "mcpJsonConfig": "{\"mcpServers\": {\"service\": {\"url\": \"http://127.0.0.1:8000/mcp\"}}}",
  "mcpType": "Persistent"
}
```

系统会自动：
1. 解析配置中的 URL
2. 向 URL 发送探测请求
3. 根据响应判断协议类型
4. 使用检测到的协议连接后端服务

### 方式二：手动指定

如果你确定后端协议类型，可以手动指定以跳过检测：

```http
POST http://localhost:8085/mcp/sse/check_status
Content-Type: application/json

{
  "mcpId": "my-service",
  "mcpJsonConfig": "{\"mcpServers\": {\"service\": {\"url\": \"http://127.0.0.1:8000/mcp\"}}}",
  "mcpType": "Persistent",
  "backendProtocol": "Stream"
}
```

## 检测逻辑

### Streamable HTTP 检测

```rust
// 发送探测请求
POST {url}
Accept: application/json, text/event-stream
Content-Type: application/json
Body: {"jsonrpc":"2.0","id":"probe","method":"ping","params":{}}

// 检查响应
if response.headers.contains("mcp-session-id") {
    return Streamable HTTP
}
if response.status == 406 Not Acceptable {
    return Streamable HTTP
}
if response.content_type.contains("text/event-stream") {
    return Streamable HTTP
}
```

### SSE 检测

```rust
// 发送探测请求
GET {url}
Accept: text/event-stream

// 检查响应
if response.content_type.contains("text/event-stream") {
    return SSE
}
```

## 日志输出

启用自动检测后，你会在日志中看到：

```
INFO  开始自动检测 MCP 服务协议: http://127.0.0.1:8000/mcp
DEBUG 尝试检测 Streamable HTTP 协议: http://127.0.0.1:8000/mcp
DEBUG 发现 mcp-session-id 头，确认为 Streamable HTTP 协议
INFO  检测到 Streamable HTTP 协议: http://127.0.0.1:8000/mcp
INFO  自动检测到后端协议: Stream for MCP ID: my-service
```

## 性能考虑

- 协议检测会增加首次连接的延迟（约 1-5 秒）
- 检测结果不会被缓存，每次启动服务都会重新检测
- 如果你的服务协议固定，建议手动指定 `backendProtocol` 以提高性能

## 错误处理

如果协议检测失败：
1. 系统会记录警告日志
2. 默认使用客户端协议作为后端协议
3. 服务仍会尝试启动，但可能连接失败

## 示例场景

### 场景 1：透明代理 Streamable HTTP 服务

```http
# 客户端使用 SSE，后端自动检测为 Streamable HTTP
POST http://localhost:8085/mcp/sse/check_status
{
  "mcpId": "streamable-service",
  "mcpJsonConfig": "{\"mcpServers\": {\"s\": {\"url\": \"http://remote:8000/mcp\"}}}",
  "mcpType": "Persistent"
}

# 客户端连接
GET http://localhost:8085/mcp/sse/proxy/streamable-service/sse
```

### 场景 2：命令行启动的服务

```http
# 命令行服务默认使用 SSE 协议（stdio 传输）
POST http://localhost:8085/mcp/sse/check_status
{
  "mcpId": "local-service",
  "mcpJsonConfig": "{\"mcpServers\": {\"s\": {\"command\": \"npx\", \"args\": [\"@modelcontextprotocol/server-filesystem\"]}}}",
  "mcpType": "Persistent"
}
```

## 限制

1. 只支持 URL 配置的自动检测
2. 命令行启动的服务默认使用 SSE 协议
3. 检测超时时间为 5 秒
4. 不支持需要认证的服务检测（会在实际连接时处理认证）

## 未来改进

- [ ] 缓存检测结果
- [ ] 支持自定义检测超时时间
- [ ] 支持更多协议类型
- [ ] 支持认证服务的检测
