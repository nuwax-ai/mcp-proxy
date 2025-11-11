#!/bin/bash

MCP_ID="test-sse-stream"
BASE_URL="http://localhost:8085"

echo "=== 建立 SSE 连接 ==="
curl -N "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream" > /tmp/sse_test.txt &
SSE_PID=$!
echo "SSE PID: $SSE_PID"

sleep 3

echo ""
echo "=== 提取 sessionId ==="
SESSION_ID=$(grep "sessionId=" /tmp/sse_test.txt | head -1 | sed 's/.*sessionId=\([^ ]*\).*/\1/')
echo "Session ID: $SESSION_ID"

echo ""
echo "=== 发送 initialize ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-1",
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {"name": "test", "version": "1.0"}
    }
  }' &

sleep 3

echo ""
echo "=== 发送 tools/list ==="
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }' &

sleep 5

echo ""
echo "=== SSE 接收到的消息 ==="
cat /tmp/sse_test.txt

kill $SSE_PID 2>/dev/null || true
rm -f /tmp/sse_test.txt
