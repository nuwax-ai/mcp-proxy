#!/bin/bash

MCP_ID="sse-to-stream-test"
BASE_URL="http://localhost:8085"

echo "=== 测试 SSE 客户端 → Streamable HTTP 后端 ==="
echo ""

# 1. 建立 SSE 连接
echo "1. 建立 SSE 连接..."
curl -N "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream" > /tmp/sse_output.txt 2>&1 &
SSE_PID=$!
echo "   SSE PID: $SSE_PID"

sleep 3

# 2. 提取 sessionId
echo ""
echo "2. 提取 sessionId..."
SESSION_ID=$(grep "sessionId=" /tmp/sse_output.txt | head -1 | sed 's/.*sessionId=\([^ ]*\).*/\1/')
echo "   Session ID: $SESSION_ID"

if [ -z "$SESSION_ID" ]; then
    echo "   ❌ 未能获取 sessionId"
    kill $SSE_PID 2>/dev/null
    exit 1
fi

# 3. 发送 initialize
echo ""
echo "3. 发送 initialize 消息..."
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-1",
    "method": "initialize",
    "params": {
      "protocolVersion": "2024-11-05",
      "capabilities": {},
      "clientInfo": {"name": "test-client", "version": "1.0"}
    }
  }' > /dev/null 2>&1 &

sleep 3

# 4. 发送 tools/list
echo ""
echo "4. 发送 tools/list 消息..."
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }' > /dev/null 2>&1 &

sleep 3

# 5. 发送 tools/call
echo ""
echo "5. 发送 tools/call 消息..."
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-3",
    "method": "tools/call",
    "params": {
      "name": "hello",
      "arguments": {"name": "World"}
    }
  }' > /dev/null 2>&1 &

sleep 3

# 6. 显示结果
echo ""
echo "=== SSE 接收到的所有消息 ==="
cat /tmp/sse_output.txt
echo ""

# 7. 解析并显示关键信息
echo ""
echo "=== 测试结果摘要 ==="
if grep -q "msg-1" /tmp/sse_output.txt; then
    echo "✅ Initialize 成功"
    grep "msg-1" /tmp/sse_output.txt | grep -o '"serverInfo":{[^}]*}' | head -1
fi

if grep -q "msg-2" /tmp/sse_output.txt; then
    echo "✅ Tools/list 成功"
    grep "msg-2" /tmp/sse_output.txt | grep -o '"tools":\[[^]]*\]' | head -1
fi

if grep -q "msg-3" /tmp/sse_output.txt; then
    echo "✅ Tools/call 成功"
    grep "msg-3" /tmp/sse_output.txt | grep -o '"content":\[[^]]*\]' | head -1
fi

# 清理
kill $SSE_PID 2>/dev/null
rm -f /tmp/sse_output.txt

echo ""
echo "=== 测试完成 ==="
