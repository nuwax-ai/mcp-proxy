#!/bin/bash

# 测试 SSE 连接和消息发送

MCP_ID="test-streamable-new"
BASE_URL="http://localhost:8085"

echo "=== 测试 SSE 连接 ==="
echo "启动 SSE 连接（后台运行）..."

# 启动 SSE 连接并保存到文件
curl -N -s "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream" > /tmp/sse_output.txt &
SSE_PID=$!

echo "SSE 连接已启动，PID: ${SSE_PID}"
sleep 2

echo ""
echo "=== 发送 initialize 消息 ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
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

echo ""
sleep 2

echo ""
echo "=== 发送 tools/list 消息 ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }'

echo ""
sleep 2

echo ""
echo "=== SSE 接收到的消息 ==="
cat /tmp/sse_output.txt

# 清理
kill ${SSE_PID} 2>/dev/null
rm -f /tmp/sse_output.txt

echo ""
echo "=== 测试完成 ==="
