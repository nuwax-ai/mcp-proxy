#!/usr/bin/env python3
"""本地 SSE MCP 测试服务器 - 用于验证 mcp-proxy 的协议探测和健康检查

支持可选的 Bearer Token 鉴权，用于复现/验证「SSE + header 鉴权 → 协议探测误判」场景：
  - 不带 --token 启动：开放服务
  - --token <T> 启动：所有请求需带 `Authorization: Bearer <T>`，否则返回 401

启动示例（uv）：
  # 开放服务
  uv run --with mcp --with uvicorn scripts/test-sse-server.py --port 8765
  # 带 Bearer 鉴权
  uv run --with mcp --with uvicorn scripts/test-sse-server.py --port 8765 --token secret123
"""
import sys

try:
    from mcp.server.fastmcp import FastMCP
except ImportError:
    print(
        "需要安装 mcp 包: uv run --with mcp --with uvicorn scripts/test-sse-server.py ...",
        file=sys.stderr,
    )
    sys.exit(1)

mcp = FastMCP("TestSSEServer")

# ---- 注册几个测试工具 ----
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

    模拟「SSE 服务依赖 Bearer 鉴权」的真实场景。包在 FastMCP 的 Starlette app
    外层，对 SSE 探测（GET /sse）和消息收发（POST）一视同仁——这正是 mcp-proxy
    协议探测在「缺 Bearer 前缀」时会误判协议的根因所在。
    """

    def __init__(self, app, token: str):
        self.app = app
        self.expected = f"Bearer {token}"

    async def __call__(self, scope, receive, send):
        # 非 http 请求（如 lifespan）直接放行
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

    parser = argparse.ArgumentParser(description="本地 SSE MCP 测试服务器（可选 Bearer 鉴权）")
    parser.add_argument("--host", default="0.0.0.0", help="监听地址 (默认: 0.0.0.0)")
    parser.add_argument("--port", type=int, default=8765, help="监听端口 (默认: 8765)")
    parser.add_argument(
        "--token",
        default=None,
        help="启用 Bearer 鉴权，请求需带 Authorization: Bearer <token>",
    )
    args = parser.parse_args()

    import uvicorn

    app = mcp.sse_app()
    if args.token:
        app = BearerAuth(app, args.token)

    sse_url = f"http://{args.host}:{args.port}/sse"
    print(f"🚀 启动 SSE MCP 测试服务器: {sse_url}")
    if args.token:
        print(f"🔐 Bearer 鉴权已启用，期望: Authorization: Bearer {args.token}")
        print(f"   带鉴权探测(应识别 SSE): mcp-proxy detect {sse_url} -H \"Authorization=Bearer {args.token}\"")
        print(f"   无鉴权探测(应兜底 Stream): mcp-proxy detect {sse_url}")
    else:
        print(f"   探测命令: mcp-proxy detect {sse_url}")
    print(f"   健康检查: mcp-proxy health {sse_url}")
    print()
    print("提示: 用 Ctrl+C 停止服务")
    print()

    uvicorn.run(app, host=args.host, port=args.port)
