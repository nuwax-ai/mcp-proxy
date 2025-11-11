#!/bin/bash

MCP_ID="test-streamable-new"
BASE_URL="http://localhost:8085"

echo "=== 测试 1: 检查服务状态 ==="
curl -s "${BASE_URL}/mcp/check/status/${MCP_ID}" | jq .

echo ""
echo "=== 测试 2: 尝试直接调用 list_tools（通过透明代理） ==="
echo "注意：这个测试是为了验证 ProxyHandler 是否工作"

# 直接测试后端服务
echo ""
echo "=== 测试 3: 直接测试后端 Streamable HTTP 服务 ==="
curl -s -X POST http://127.0.0.1:8000/mcp \
  -H "Accept: application/json, text/event-stream" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":"test","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' \
  | head -5

echo ""
echo ""
echo "=== 测试 4: SSE 连接测试 ==="
echo "提示：SSE 连接会保持打开状态，按 Ctrl+C 停止"
echo "在另一个终端运行以下命令发送消息："
echo ""
echo "curl -X POST ${BASE_URL}/mcp/sse/proxy/${MCP_ID}/message \\"
echo "  -H 'Content-Type: application/json' \\"
echo "  -d '{\"jsonrpc\":\"2.0\",\"id\":\"msg-1\",\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1.0\"}}}'"
echo ""
echo "开始 SSE 连接..."
curl -N "${BASE_URL}/mcp/sse/proxy/${MCP_ID}/sse" \
  -H "Accept: text/event-stream"
