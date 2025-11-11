#!/usr/bin/env python3
"""
测试 SSE MCP 客户端
"""
import json
import requests
import sseclient
import threading
import time

MCP_ID = "test-sse-stream"
BASE_URL = "http://localhost:8085"
SSE_URL = f"{BASE_URL}/mcp/sse/proxy/{MCP_ID}/sse"
MESSAGE_URL_TEMPLATE = f"{BASE_URL}/mcp/sse/proxy/{MCP_ID}/message"
MESSAGE_URL = None  # 将在获取 sessionId 后设置

def listen_sse():
    """监听 SSE 事件"""
    global MESSAGE_URL
    print("=== 开始监听 SSE 连接 ===")
    try:
        response = requests.get(SSE_URL, headers={'Accept': 'text/event-stream'}, stream=True)
        client = sseclient.SSEClient(response)
        
        for event in client.events():
            print(f"\n收到 SSE 事件:")
            print(f"  Event: {event.event}")
            print(f"  Data: {event.data}")
            
            # 如果是 endpoint 事件，提取 sessionId
            if event.event == "endpoint":
                MESSAGE_URL = f"{BASE_URL}{event.data}"
                print(f"  ✅ 获取到 MESSAGE_URL: {MESSAGE_URL}")
            
            # 尝试解析 JSON
            try:
                data = json.loads(event.data)
                print(f"  解析后: {json.dumps(data, indent=2, ensure_ascii=False)}")
            except:
                pass
                
    except Exception as e:
        print(f"SSE 连接错误: {e}")

def send_message(msg_id, method, params=None):
    """发送消息到 MCP 服务"""
    message = {
        "jsonrpc": "2.0",
        "id": msg_id,
        "method": method,
        "params": params or {}
    }
    
    print(f"\n=== 发送消息: {method} ===")
    print(json.dumps(message, indent=2, ensure_ascii=False))
    
    try:
        response = requests.post(
            MESSAGE_URL,
            json=message,
            headers={'Content-Type': 'application/json'},
            timeout=5
        )
        print(f"响应状态码: {response.status_code}")
        if response.text:
            print(f"响应内容: {response.text}")
    except requests.exceptions.Timeout:
        print("请求超时（这是正常的，响应会通过 SSE 返回）")
    except Exception as e:
        print(f"发送消息错误: {e}")

def main():
    global MESSAGE_URL
    # 启动 SSE 监听线程
    sse_thread = threading.Thread(target=listen_sse, daemon=True)
    sse_thread.start()
    
    # 等待 SSE 连接建立并获取 sessionId
    print("等待获取 sessionId...")
    timeout = time.time() + 10
    while MESSAGE_URL is None and time.time() < timeout:
        time.sleep(0.5)
    
    if MESSAGE_URL is None:
        print("❌ 未能获取 sessionId，退出")
        return
    
    print(f"✅ 已获取 MESSAGE_URL: {MESSAGE_URL}")
    time.sleep(1)
    
    # 发送 initialize 消息
    send_message("msg-1", "initialize", {
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {
            "name": "test-client",
            "version": "1.0.0"
        }
    })
    
    time.sleep(2)
    
    # 发送 tools/list 消息
    send_message("msg-2", "tools/list", {})
    
    time.sleep(2)
    
    print("\n=== 测试完成 ===")

if __name__ == "__main__":
    main()
