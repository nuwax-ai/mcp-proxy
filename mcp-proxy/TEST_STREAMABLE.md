# 测试 Streamable HTTP MCP 服务

## 前提条件

1. **启动 mcp-proxy 服务**
   ```bash
   cd /Volumes/soddygo/git_work/mcp-proxy
   cargo run -p mcp-proxy --bin mcp-proxy
   ```
   默认端口: 8080

2. **启动您的 Streamable HTTP MCP 服务**
   ```bash
   # 您的服务地址
   http://0.0.0.0:8000/mcp
   ```

## 测试步骤

### 步骤 1: 检查服务状态
```bash
curl -X POST http://localhost:8080/mcp/sse/check_status \
  -H "Content-Type: application/json" \
  -d '{
    "mcpId": "test-streamable-service",
    "mcpJsonConfig": "{\"mcpServers\": {\"test-service\": {\"url\": \"http://0.0.0.0:8000/mcp\"}}}",
    "mcpType": "Persistent",
    "mcpProtocol": "Stream"
  }'
```

期望响应:
```json
{
  "ready": true,
  "status": "Ready",
  "message": null
}
```

### 步骤 2: 建立 SSE 连接
```bash
curl -N http://localhost:8080/mcp/sse/proxy/test-streamable-service/sse \
  -H "Accept: text/event-stream"
```

这个连接会保持打开，接收来自远程 Streamable 服务的实时更新。

### 步骤 3: 发送初始化消息
在新终端中执行:
```bash
curl -X POST http://localhost:8080/mcp/sse/proxy/test-streamable-service/message \
  -H "Content-Type: application/json" \
  -d '{
    "id": "msg-1",
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {
        "name": "test-client",
        "version": "1.0.0"
      }
    }
  }'
```

### 步骤 4: 列出工具
```bash
curl -X POST http://localhost:8080/mcp/sse/proxy/test-streamable-service/message \
  -H "Content-Type: application/json" \
  -d '{
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }'
```

### 步骤 5: 调用工具
```bash
curl -X POST http://localhost:8080/mcp/sse/proxy/test-streamable-service/message \
  -H "Content-Type: application/json" \
  -d '{
    "id": "msg-3",
    "method": "tools/call",
    "params": {
      "name": "your-tool-name",
      "arguments": {}
    }
  }'
```

## 使用 REST Client

如果使用 VSCode REST Client 扩展:
1. 打开 `test_mcp_streamable.rest` 文件
2. 按 `Ctrl+Alt+R` (Mac: `Cmd+Alt+R`) 发送请求
3. 按顺序执行请求

## 透明代理说明

```
用户 (SSE 接口)
    ↓
mcp-proxy (端口 8080)
    ↓
Streamable HTTP 客户端
    ↓
远程服务 (端口 8000)
```

- 用户通过 SSE 协议访问 mcp-proxy
- mcp-proxy 内部使用 Streamable HTTP 协议连接远程服务
- 实现协议透明转换

## 故障排除

### 1. 连接失败
```bash
# 检查服务是否启动
curl http://0.0.0.0:8000/mcp/health
```

### 2. 协议不匹配
- 确保 mcpProtocol 字段设置为 "Stream"
- 远程服务必须支持 Streamable HTTP 协议

### 3. 认证问题
如果远程服务需要认证:
```json
{
  "mcpId": "test-streamable-service",
  "mcpJsonConfig": "{
    \"mcpServers\": {
      \"test-service\": {
        \"url\": \"http://0.0.0.0:8000/mcp\",
        \"authToken\": \"your-token\"
      }
    }
  }",
  "mcpType": "Persistent",
  "mcpProtocol": "Stream"
}
```

## 查看日志

在 mcp-proxy 启动的终端中查看日志:
```
[INFO] 创建Streamable HTTP客户端连接到: http://0.0.0.0:8000/mcp
[INFO] Streamable HTTP客户端已启动，MCP ID: test-streamable-service, 类型: Persistent
```
