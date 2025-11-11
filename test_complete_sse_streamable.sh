#!/bin/bash

set -e

MCP_ID="test-streamable-service"
BASE_URL="http://localhost:8085"

echo "========================================="
echo "测试 SSE 客户端 → Streamable HTTP 后端"
echo "========================================="

# 1. 创建服务
echo ""
echo "1. 创建服务..."
RESPONSE=$(curl -s -X POST "${BASE_URL}/mcp/sse/check_status" \
  -H "Content-Type: application/json" \
  -d "{
    \"mcpId\": \"${MCP_ID}\",
    \"mcpJsonConfig\": \"{\\\"mcpServers\\\": {\\\"test\\\": {\\\"url\\\": \\\"http://127.0.0.1:8000/mcp\\\"}}}\",
    \"mcpType\": \"Persistent\"
  }")

echo "$RESPONSE" | jq .

# 2. 等待服务就绪
echo ""
echo "2. 等待服务就绪..."
sleep 5

STATUS=$(curl -s "${BASE_URL}/mcp/check/status/${MCP_ID}" | jq -r '.data.status')
echo "   服务状态: ${STATUS}"

if [ "${STATUS}" != "Ready" ]; then
    echo "   ❌ 服务未就绪"
    exit 1
fi

echo "   ✅ 服务已就绪"

# 3. 建立 SSE 连接
echo ""
echo "3. 建立 SSE 连接..."
curl -N "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream" > /tmp/sse_test_output.txt 2>&1 &
SSE_PID=$!
echo "   SSE PID: ${SSE_PID}"

sleep 3

# 4. 提取 sessionId
echo ""
echo "4. 提取 sessionId..."
SESSION_ID=$(grep "sessionId=" /tmp/sse_test_output.txt | head -1 | sed 's/.*sessionId=\([^ ]*\).*/\1/')
echo "   Session ID: ${SESSION_ID}"

if [ -z "${SESSION_ID}" ]; then
    echo "   ❌ 未能获取 sessionId"
    kill ${SSE_PID} 2>/dev/null
    exit 1
fi

echo "   ✅ 成功获取 sessionId"

# 5. 发送 initialize
echo ""
echo "5. 发送 initialize 消息..."
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

# 6. 发送 tools/list
echo ""
echo "6. 发送 tools/list 消息..."
curl -s -X POST "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message?sessionId=${SESSION_ID}" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": "msg-2",
    "method": "tools/list",
    "params": {}
  }' > /dev/null 2>&1 &

sleep 3

# 7. 发送 tools/call
echo ""
echo "7. 发送 tools/call 消息..."
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

# 8. 显示结果
echo ""
echo "========================================="
echo "SSE 接收到的所有消息"
echo "========================================="
cat /tmp/sse_test_output.txt
echo ""

# 9. 验证结果
echo ""
echo "========================================="
echo "测试结果验证"
echo "========================================="

if grep -q "msg-1" /tmp/sse_test_output.txt; then
    echo "✅ Initialize 成功"
    echo "   服务器信息:"
    grep "msg-1" /tmp/sse_test_output.txt | grep -o '"serverInfo":{[^}]*}' | head -1 | jq .
else
    echo "❌ Initialize 失败"
fi

if grep -q "msg-2" /tmp/sse_test_output.txt; then
    echo "✅ Tools/list 成功"
    TOOLS=$(grep "msg-2" /tmp/sse_test_output.txt | grep -o '"tools":\[[^]]*\]' | head -1)
    echo "   工具列表: $TOOLS"
else
    echo "❌ Tools/list 失败"
fi

if grep -q "msg-3" /tmp/sse_test_output.txt; then
    echo "✅ Tools/call 成功"
    RESULT=$(grep "msg-3" /tmp/sse_test_output.txt | grep -o '"content":\[[^]]*\]' | head -1)
    echo "   调用结果: $RESULT"
else
    echo "❌ Tools/call 失败"
fi

# 清理
kill ${SSE_PID} 2>/dev/null || true
rm -f /tmp/sse_test_output.txt

echo ""
echo "========================================="
echo "测试完成！"
echo "========================================="
