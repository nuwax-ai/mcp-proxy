# 快速测试指南

## 前提
- mcp-proxy 运行在: http://localhost:8080
- 您的 Streamable 服务运行在: http://0.0.0.0:8000/mcp

## 一键测试 (使用 curl)

### 1. 检查服务
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

### 2. 在新终端中建立 SSE 连接
```bash
curl -N http://localhost:8080/mcp/sse/proxy/test-streamable-service/sse
```

### 3. 发送初始化消息 (在第三个终端)
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

### 4. 列出工具
```bash
curl -X POST http://localhost:8080/mcp/sse/proxy/test-streamable-service/message \
  -H "Content-Type: application/json" \
  -d '{
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }'
```

## 透明代理工作流程

```
[用户] SSE 接口 (端口 8080)
    ↓
[mcp-proxy] Streamable HTTP 客户端
    ↓
[远程服务] Streamable HTTP (端口 8000)
```

- 用户通过 **SSE 协议** 访问 mcp-proxy
- mcp-proxy 使用 **Streamable HTTP 协议** 连接远程服务
- 实现协议透明转换

## 查看日志
在 mcp-proxy 启动终端中查看:
```
[INFO] 创建Streamable HTTP客户端连接到: http://0.0.0.0:8000/mcp
[INFO] Streamable HTTP客户端已启动，MCP ID: test-streamable-service, 类型: Persistent
```

## 使用 VSCode REST Client
1. 打开 `test_mcp_streamable.rest` 文件
2. 按 `Ctrl+Alt+R` (Mac: `Cmd+Alt+R`) 发送请求
3. 按顺序执行请求
