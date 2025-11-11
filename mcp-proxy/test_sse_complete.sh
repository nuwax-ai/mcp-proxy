#!/bin/bash

set -e

MCP_ID="test-sse-complete"
BASE_URL="http://localhost:8085"

echo "=== 1. 创建服务 ==="
curl -s -X POST "${BASE_URL}/mcp/sse/check_status" \
  -H "Content-Type: application/json" \
  -d "{
    \"mcpId\": \"${MCP_ID}\",
    \"mcpJsonConfig\": \"{\\\"mcpServers\\\": {\\\"test\\\": {\\\"url\\\": \\\"http://127.0.0.1:8000/mcp\\\"}}}\",
    \"mcpType\": \"Persistent\",
    \"backendProtocol\": \"Stream\"
  }" | jq .

echo ""
echo "=== 2. 等待服务就绪 ==="
sleep 5

STATUS=$(curl -s "${BASE_URL}/mcp/check/status/${MCP_ID}" | jq -r '.data.status')
echo "服务状态: ${STATUS}"

if [ "${STATUS}" != "Ready" ]; then
    echo "服务未就绪，退出"
    exit 1
fi

echo ""
echo "=== 3. 建立 SSE 连接并保存到文件 ==="
curl -N "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream" > /tmp/sse_output.txt &
SSE_PID=$!
echo "SSE 连接已启动，PID: ${SSE_PID}"

sleep 3

echo ""
echo "=== 4. 从 SSE 输出中提取 sessionId ==="
SESSION_ID=$(grep "sessionId=" /tmp/sse_output.txt | head -1 | sed 's/.*sessionId=\([^"]*\).*/\1/')
echo "Session ID: ${SESSION_ID}"

if [ -z "${SESSION_ID}" ]; then
    echo "未能获取 session ID"
    kill ${SSE_PID} 2>/dev/null
    exit 1
fi

echo ""
echo "=== 5. 发送 initialize 消息 ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-init",
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

sleep 2

echo ""
echo "=== 6. 发送 tools/list 消息 ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-tools",
    "method": "tools/list",
    "params": {}
  }'

sleep 3

echo ""
echo "=== 7. 查看 SSE 接收到的消息 ==="
cat /tmp/sse_output.txt

echo ""
echo "=== 8. 清理 ==="
kill ${SSE_PID} 2>/dev/null || true
rm -f /tmp/sse_output.txt

echo ""
echo "=== 测试完成 ==="
