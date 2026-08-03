#!/usr/bin/env python3
"""本地 Streamable HTTP MCP 测试服务器 - 用于验证 mcp-proxy 的 streamable 路径

与 test-sse-server.py 对称，区别在于用 Streamable HTTP（rmcp 3.1.0 默认协议）
而非 SSE。这条路径会真正走 mcp-streamable-proxy（官方 rmcp 3.1.0）的客户端逻辑，
是 rmcp 迁移后最关键的集成验证。

支持可选的 Bearer Token 鉴权：
  - 不带 --token 启动：开放服务
  - --token <T> 启动：所有请求需带 `Authorization: Bearer <T>`，否则返回 401

启动示例（uv）：
  uv run --with fastmcp --with uvicorn scripts/test-streamable-server.py --port 9732
  uv run --with fastmcp --with uvicorn scripts/test-streamable-server.py --port 9732 --token secret123
"""
import sys

try:
    from fastmcp import FastMCP
except ImportError:
    print(
        "需要安装 fastmcp 包: uv run --with fastmcp --with uvicorn scripts/test-streamable-server.py ...",
        file=sys.stderr,
    )
    sys.exit(1)

mcp = FastMCP("TestStreamableServer")

# ---- 与 SSE 测试服务器相同的 3 个工具，便于对比 ----
@mcp.tool()
def echo(text: str) -> str:
    """回显输入的文本"""
    return f"[Echo] {text}"

@mcp.tool()
def add(a: int, b: int) -> int:
    """两数相加"""
    return a + b

@mcp.tool()
def get_server_time() -> str:
    """获取服务器当前时间"""
    from datetime import datetime

    return datetime.now().isoformat()


class BearerAuth:
    """纯 ASGI 中间件：校验 `Authorization: Bearer <token>`，不匹配返回 401。

    包在 FastMCP 的 Starlette app 外层，对 streamable HTTP 的 POST/GET 一视同仁。
    """

    def __init__(self, app, token: str):
        self.app = app
        self.expected = f"Bearer {token}"

    async def __call__(self, scope, receive, send):
        if scope.get("type") != "http":
            await self.app(scope, receive, send)
            return

        auth = ""
        for key, value in scope.get("headers") or []:
            if key.decode("latin-1").lower() == "authorization":
                auth = value.decode("latin-1")
                break

        if auth == self.expected:
            await self.app(scope, receive, send)
        else:
            await send(
                {
                    "type": "http.response.start",
                    "status": 401,
                    "headers": [[b"content-type", b"application/json"]],
                }
            )
            await send(
                {
                    "type": "http.response.body",
                    "body": b'{"error":"unauthorized: missing or invalid Authorization header"}',
                }
            )


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="本地 Streamable HTTP MCP 测试服务器（可选 Bearer 鉴权）")
    parser.add_argument("--host", default="127.0.0.1", help="监听地址 (默认: 127.0.0.1)")
    parser.add_argument("--port", type=int, default=9732, help="监听端口 (默认: 9732)")
    parser.add_argument(
        "--token",
        default=None,
        help="启用 Bearer 鉴权，请求需带 Authorization: Bearer <token>",
    )
    args = parser.parse_args()

    import uvicorn

    # streamable HTTP 挂载在 /mcp（MCP 2025+ 约定的默认端点）
    app = mcp.http_app(path="/mcp")
    if args.token:
        app = BearerAuth(app, args.token)

    url = f"http://{args.host}:{args.port}/mcp"
    print(f"🚀 启动 Streamable HTTP MCP 测试服务器: {url}")
    if args.token:
        print(f"🔐 Bearer 鉴权已启用，期望: Authorization: Bearer {args.token}")
        print(f'   探测: mcp-proxy detect {url} -a "Bearer {args.token}"')
        print(f'   健康检查: mcp-proxy health {url} -a "Bearer {args.token}"')
    else:
        print(f"   探测命令: mcp-proxy detect {url}")
        print(f"   健康检查: mcp-proxy health {url}")
    print()
    print("提示: 用 Ctrl+C 停止服务")
    print()

    uvicorn.run(app, host=args.host, port=args.port)
