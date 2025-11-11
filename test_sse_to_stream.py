#!/usr/bin/env python3
"""测试 SSE 客户端连接到 Streamable HTTP 后端"""

import requests
import json
import sseclient
import time

MCP_ID = "test-sse-stream"
BASE_URL = "http://localhost:8085"

def test_sse_to_stream():
    print("=" * 60)
    print("测试 SSE 客户端 → Streamable HTTP 后端")
    print("=" * 60)
    
    # 1. 建立 SSE 连接
    print("\n1. 建立 SSE 连接...")
    sse_url = f"{BASE_URL}/mcp/sse/proxy/{MCP_ID}/sse"
    print(f"   URL: {sse_url}")
    
    response = requests.get(sse_url, stream=True, headers={"Accept": "text/event-stream"})
    client = sseclient.SSEClient(response)
    
    # 获取 endpoint 事件
    endpoint_url = None
    for event in client.events():
        print(f"   收到事件: {event.event}")
        print(f"   数据: {event.data}")
        if event.event == "endpoint":
            endpoint_url = event.data
            break
    
    if not endpoint_url:
        print("   ❌ 未能获取 endpoint")
        return
    
    print(f"   ✅ 获取到 endpoint: {endpoint_url}")
    
    # 2. 发送 initialize 消息
    print("\n2. 发送 initialize 消息...")
    message_url = f"{BASE_URL}{endpoint_url}"
    print(f"   URL: {message_url}")
    
    initialize_msg = {
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
    }
    
    print(f"   发送: {json.dumps(initialize_msg, indent=2)}")
    resp = requests.post(message_url, json=initialize_msg)
    print(f"   状态码: {resp.status_code}")
    print(f"   响应: {resp.text}")
    
    # 3. 从 SSE 流中读取响应
    print("\n3. 从 SSE 流中读取响应...")
    timeout = time.time() + 5  # 5秒超时
    for event in client.events():
        if time.time() > timeout:
            print("   ⏱️  超时")
            break
        
        print(f"   事件类型: {event.event}")
        if event.event == "message":
            print(f"   ✅ 收到消息: {event.data}")
            msg = json.loads(event.data)
            if msg.get("id") == "msg-1":
                print(f"   ✅ Initialize 成功!")
                print(f"   服务器信息: {json.dumps(msg.get('result', {}), indent=2)}")
                break
    
    # 4. 发送 tools/list 消息
    print("\n4. 发送 tools/list 消息...")
    tools_msg = {
        "jsonrpc": "2.0",
        "id": "msg-2",
        "method": "tools/list",
        "params": {}
    }
    
    print(f"   发送: {json.dumps(tools_msg, indent=2)}")
    resp = requests.post(message_url, json=tools_msg)
    print(f"   状态码: {resp.status_code}")
    
    # 5. 从 SSE 流中读取 tools/list 响应
    print("\n5. 从 SSE 流中读取 tools/list 响应...")
    timeout = time.time() + 5
    for event in client.events():
        if time.time() > timeout:
            print("   ⏱️  超时")
            break
        
        print(f"   事件类型: {event.event}")
        if event.event == "message":
            print(f"   收到消息: {event.data}")
            msg = json.loads(event.data)
            if msg.get("id") == "msg-2":
                print(f"   ✅ Tools/list 成功!")
                tools = msg.get("result", {}).get("tools", [])
                print(f"   工具数量: {len(tools)}")
                for tool in tools:
                    print(f"     - {tool.get('name')}: {tool.get('description')}")
                break
    
    print("\n" + "=" * 60)
    print("测试完成!")
    print("=" * 60)

if __name__ == "__main__":
    test_sse_to_stream()
