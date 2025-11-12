# RMCP-PROXY

## 项目简介

本项目包含两个主要服务：

### Document Parser Service (文档解析服务)
一个高性能的多格式文档解析服务，支持PDF、Word、Excel、PowerPoint等格式转换为结构化Markdown。

**主要特性:**
- 🔄 多引擎解析：MinerU (PDF) + MarkItDown (其他格式)
- 🐍 智能Python环境管理：使用当前目录虚拟环境 (./venv/)
- 📊 实时结构化处理：自动生成目录和章节
- ☁️ OSS集成：支持阿里云对象存储
- 🚀 异步任务处理：支持大文件批量处理

### MCP Proxy Service (MCP代理服务)
实现了一个 mcp 代理服务，用户可以通过 SSE（Server-Sent Events）协议或 Streamable HTTP 协议，配置我们提供的 URL 地址，远程使用服务器提供的 mcp 功能。

**主要功能:**
- 支持通过 SSE 协议与客户端通信，实时推送数据
- 支持 Streamable HTTP 协议，实现高效的双向通信
- 支持动态添加 mcp 插件：只需在 mcp 社区查找所需插件，复制对应的 JSON 配置，粘贴到本服务的配置中，即可自动加载并启用插件
- 支持协议转换：可以将 SSE 后端转换为 Streamable HTTP 前端，反之亦然
- 支持自动协议检测：根据 URL 路径和配置自动识别协议类型
- 每个插件配置完成后，服务器会自动启动对应的 mcp 服务，并生成可供访问的 URL 地址
- 支持持续运行和一次性任务两种模式

## 🎯 MCP Proxy 快速开始

### 1. 启动 MCP 代理服务

```bash
# 启动 MCP 代理服务器（默认端口 8080）
mcp-proxy

# 指定端口启动
mcp-proxy --port 8080

# 或使用标准方式启动
cd mcp-proxy
cargo run --bin mcp-proxy
```

### 2. 添加 MCP 服务

**添加 SSE MCP 服务：**
```bash
curl -X POST http://localhost:8080/mcp/sse/add_route \
  -H "Content-Type: application/json" \
  -d '{
    "mcp_json_config": "{\"mcpServers\": {\"playwright\": {\"command\": \"npx\", \"args\": [\"@playwright/mcp@latest\", \"--headless\"]}}}",
    "mcp_type": "Persistent"
  }'
```

**添加 Streamable HTTP MCP 服务：**
```bash
curl -X POST http://localhost:8080/mcp/stream/add_route \
  -H "Content-Type: application/json" \
  -d '{
    "mcp_json_config": "{\"mcpServers\": {\"example\": {\"url\": \"https://example.com/mcp\", \"type\": \"stream\"}}}",
    "mcp_type": "Persistent"
  }'
```

### 3. 检查 MCP 服务状态

```bash
curl -X POST http://localhost:8080/mcp/sse/check_status \
  -H "Content-Type: application/json" \
  -d '{
    "mcpId": "服务ID",
    "mcpJsonConfig": "MCP JSON 配置",
    "mcpType": "OneShot"
  }'
```

### 4. 连接 MCP 服务

**SSE 协议连接：**
```
http://localhost:8080/mcp/sse/proxy/{mcpId}/sse
```

**Streamable HTTP 协议连接：**
```
http://localhost:8080/mcp/stream/proxy/{mcpId}
```

### 5. 删除 MCP 服务

```bash
curl -X DELETE http://localhost:8080/mcp/{mcpId}
```

## 🔧 MCP Proxy API 详细说明

### 1. 添加 MCP 服务接口

#### SSE 服务
```
POST http://localhost:8080/mcp/sse/add_route
Content-Type: application/json
```

#### Streamable HTTP 服务
```
POST http://localhost:8080/mcp/stream/add_route
Content-Type: application/json
```

**请求参数：**
```json
{
  "mcp_json_config": "MCP服务的JSON配置字符串",
  "mcp_type": "Persistent"  // 或 "OneShot"
}
```

**支持的 MCP 配置格式：**

*命令行配置：*
```json
{
  "mcpServers": {
    "my-service": {
      "command": "npx",
      "args": ["-y", "@playwright/mcp@latest", "--headless"],
      "env": {
        "API_KEY": "your-api-key"
      }
    }
  }
}
```

*URL 配置（SSE）：*
```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp/sse",
      "type": "sse",
      "headers": {
        "Authorization": "Bearer your-token"
      }
    }
  }
}
```

*URL 配置（Streamable HTTP）：*
```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp",
      "type": "stream",  // 或 "http"
      "headers": {
        "Authorization": "Bearer your-token"
      }
    }
  }
}
```

*URL 配置（自动检测）：*
```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp"
      // 不指定 type 字段，系统将自动检测
    }
  }
}
```

**响应示例：**
```json
{
  "success": true,
  "data": {
    "mcp_id": "abc123",
    "sse_path": "/mcp/sse/proxy/abc123/sse",
    "message_path": "/mcp/sse/proxy/abc123/message",
    "mcp_type": "Persistent"
  }
}
```

### 2. MCP 服务状态检查接口

```
POST http://localhost:8080/mcp/sse/check_status
POST http://localhost:8080/mcp/stream/check_status
Content-Type: application/json
```

**请求参数：**
```json
{
  "mcpId": "服务唯一标识符",
  "mcpJsonConfig": "MCP服务的JSON配置",
  "mcpType": "Persistent",  // 或 "OneShot"
  "backendProtocol": "sse"  // 可选，指定后端协议
}
```

**响应示例：**
```json
{
  "success": true,
  "data": {
    "ready": true,
    "status": "READY",  // READY | PENDING | ERROR
    "message": null
  }
}
```

### 3. SSE 协议连接接口

建立 SSE 连接后，客户端可以：
- 接收实时事件推送
- 发送 MCP 消息请求

**SSE 端点：**
```
GET http://localhost:8080/mcp/sse/proxy/{mcpId}/sse
```

**消息发送端点：**
```
POST http://localhost:8080/mcp/sse/proxy/{mcpId}/message
Content-Type: application/json

{
  "jsonrpc": "2.0",
  "id": "msg_123",
  "method": "tools/list",
  "params": {}
}
```

### 4. Streamable HTTP 协议连接接口

**Streamable HTTP 端点：**
```
GET  http://localhost:8080/mcp/stream/proxy/{mcpId}  # 获取服务信息
POST http://localhost:8080/mcp/stream/proxy/{mcpId}  # 发送消息
Content-Type: application/json
```

**请求示例：**
```json
{
  "jsonrpc": "2.0",
  "id": "msg_123",
  "method": "tools/list",
  "params": {}
}
```

### 5. 删除 MCP 服务接口

```
DELETE http://localhost:8080/mcp/{mcpId}
```

**响应示例：**
```json
{
  "success": true,
  "data": {
    "mcp_id": "abc123",
    "message": "已删除路由: abc123"
  }
}
```

## 🔧 MCP Proxy 协议支持

### 支持的协议类型

| 协议类型 | 前端支持 | 后端支持 | 别名 | 说明 |
|---------|---------|---------|------|------|
| **Stdio** | ❌ | ✅ | - | 标准输入输出，用于命令行启动的服务 |
| **SSE** | ✅ | ✅ | - | Server-Sent Events，单向实时通信 |
| **Streamable HTTP** | ✅ | ✅ | `http`, `stream` | 高效的双向通信协议 |

### 协议转换

MCP Proxy 支持在不同协议之间进行转换：

- **SSE → Streamable**：将 SSE 后端服务转换为 Streamable HTTP 前端
- **Streamable → SSE**：将 Streamable HTTP 后端服务转换为 SSE 前端
- **Stdio → SSE**：将命令行启动的服务转换为 SSE 前端
- **Stdio → Streamable**：将命令行启动的服务转换为 Streamable HTTP 前端

### 自动协议检测

当 URL 配置中未指定 `type` 字段时，系统会自动检测协议：

```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp/sse"
      // 系统将自动检测为 SSE 协议
    }
  }
}
```

如果检测失败，可以显式指定类型：
```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp",
      "type": "sse"  // 显式指定协议
    }
  }
}
```

## 🎯 MCP Proxy 使用场景

### 场景 1：远程 MCP 服务代理

假设你有一个运行在远程服务器上的 MCP 服务：
```json
{
  "mcpServers": {
    "remote-service": {
      "url": "https://remote-server.com/mcp/sse",
      "type": "sse"
    }
  }
}
```

通过 MCP Proxy，你可以：
1. 将其暴露为本地 SSE 端点
2. 转换为 Streamable HTTP 协议
3. 添加认证、限流等中间件

### 场景 2：本地命令服务代理

将本地命令行工具包装为网络服务：
```json
{
  "mcpServers": {
    "local-db": {
      "command": "go-mcp-mysql",
      "args": ["--host", "localhost", "--user", "root"]
    }
  }
}
```

### 场景 3：协议转换桥接

连接使用不同协议的 MCP 服务：
```json
{
  "mcpServers": {
    "bridge": {
      "url": "https://sse-service.com/mcp",
      "type": "sse"
    }
  }
}
```

客户端可以使用 Streamable HTTP 协议连接，系统自动处理协议转换。

## 🔧 MCP Proxy 故障排除

### 常见问题

#### 1. 服务启动失败

**问题：端口被占用**
```bash
# 错误信息
Error: bind error: Address already in use
```

**解决方案：**
```bash
# 使用其他端口
mcp-proxy --port 8081

# 或查找占用端口的进程
lsof -i :8080  # Linux/macOS
netstat -ano | findstr :8080  # Windows
```

#### 2. MCP 服务添加失败

**问题：JSON 配置格式错误**
```bash
# 错误信息
Failed to parse MCP config: ...
```

**解决方案：**
- 检查 JSON 语法是否正确
- 确保 `mcpServers` 字段存在且格式正确
- 使用在线 JSON 验证工具检查

**正确格式：**
```json
{
  "mcpServers": {
    "service-name": {
      "command": "npx",
      "args": ["@playwright/mcp@latest"]
    }
  }
}
```

#### 3. 协议检测失败

**问题：无法自动检测协议**
```
Error: 自动检测协议失败: ...
```

**解决方案：**
1. 显式指定协议类型：
```json
{
  "mcpServers": {
    "my-service": {
      "url": "https://example.com/mcp",
      "type": "sse"  // 显式指定
    }
  }
}
```

2. 检查 URL 是否可访问：
```bash
curl -I https://example.com/mcp
```

#### 4. SSE 连接问题

**问题：SSE 连接断开**
```bash
# 检查服务端日志
tail -f logs/mcp-proxy.log
```

**解决方案：**
- 确保客户端正确处理 `text/event-stream` MIME 类型
- 检查网络连接稳定性
- 调整 SSE keep-alive 设置

#### 5. Streamable HTTP 连接问题

**问题：消息发送失败**
```bash
# 检查响应状态码和错误信息
```

**解决方案：**
- 确保请求体符合 JSON-RPC 2.0 格式
- 检查 `Content-Type: application/json` 头部
- 验证服务端点是否正确

### 环境要求

- **Rust**: 1.70+ (推荐 1.75+)
- **操作系统**: Linux, macOS, Windows
- **磁盘空间**: 至少 100MB 可用空间
- **网络**: 需要访问远程 MCP 服务

### 目录结构

```
mcp-proxy/
├── logs/                    # 日志文件
├── Cargo.toml              # 项目配置
├── src/                    # 源代码
│   ├── main.rs            # 主入口
│   ├── model/             # 数据模型
│   ├── server/            # 服务器实现
│   └── lib.rs             # 库入口
└── README.md              # 说明文档
```

## 🔧 MCP Proxy 配置说明

### 命令行参数

```bash
# 查看所有可用选项
mcp-proxy --help

# 常用选项
--port <PORT>              # 指定服务端口 (默认: 8080)
--host <HOST>              # 指定绑定地址 (默认: 0.0.0.0)
--log-level <LEVEL>        # 日志级别 (trace, debug, info, warn, error)
--workers <NUM>            # 工作线程数 (默认: CPU 核数)
```

### 环境变量

```bash
# MCP_PROXY_PORT          # 服务端口
# MCP_PROXY_HOST          # 绑定地址
# MCP_PROXY_LOG_LEVEL     # 日志级别
# RUST_LOG                # Rust 日志级别
```

### MCP 服务类型

#### Persistent（持久服务）
- 服务启动后持续运行
- 适用于需要长期运行的服务（如数据库、API 服务）
- 手动删除或服务器关闭时停止

#### OneShot（一次性任务）
- 服务执行完成后自动停止
- 适用于短期任务（如脚本执行、数据处理）
- 3分钟未访问自动清理

### 日志配置

默认日志位置：`./logs/mcp-proxy.log`

**日志级别说明：**
- `debug`: 详细的调试信息
- `info`: 一般信息记录（推荐）
- `warn`: 警告信息
- `error`: 错误信息

**日志轮转：**
- 每天自动轮转日志文件
- 保留最近7天的日志
- 压缩旧日志文件

## 🚀 Document Parser 快速开始

### 1. 环境初始化
```bash
# 进入项目目录
cd document-parser

# 初始化虚拟环境和依赖（首次使用）
document-parser uv-init

# 检查环境状态
document-parser check
```

## 🎯 Voice CLI 快速开始

### 1. 服务配置初始化

**初始化服务器配置：**
```bash
# 生成默认服务器配置文件 (./server-config.yml)
voice-cli server init

# 指定配置文件路径
voice-cli server init --config ./my-server-config.yml

# 强制覆盖已存在的配置文件
voice-cli server init --force
```

**初始化集群配置：**
```bash
# 生成默认集群配置文件 (./cluster-config.yml)
voice-cli cluster init

# 指定配置文件路径和端口
voice-cli cluster init --config ./my-cluster-config.yml --http-port 8081 --grpc-port 50052

# 强制覆盖已存在的配置文件
voice-cli cluster init --force
```

**初始化负载均衡器配置：**
```bash
# 生成默认负载均衡器配置文件 (./lb-config.yml)
voice-cli lb init

# 指定配置文件路径和端口
voice-cli lb init --config ./my-lb-config.yml --port 8091

# 强制覆盖已存在的配置文件
voice-cli lb init --force
```

### 2. 启动服务

**启动单节点服务器：**
```bash
# 使用默认配置启动
voice-cli server run

# 使用指定配置文件启动
voice-cli server run --config ./server-config.yml
```

**启动集群节点：**
```bash
# 使用默认配置启动
voice-cli cluster start

# 使用指定配置文件启动
voice-cli cluster start --config ./cluster-config.yml

# 指定节点ID和端口
voice-cli cluster start --config ./cluster-config.yml --node-id "node-1" --http-port 8081 --grpc-port 50052
```

**启动负载均衡器：**
```bash
# 使用默认配置启动
voice-cli lb start

# 使用指定配置文件启动
voice-cli lb start --config ./lb-config.yml

# 指定端口启动
voice-cli lb start --port 8091
```

### 2. 启动服务
```bash
# 启动HTTP服务器
document-parser server

# 或者指定端口
document-parser server --port 8080
```

### 3. 虚拟环境管理

**激活虚拟环境:**
```bash
# Linux/macOS
source ./venv/bin/activate

# Windows
.\venv\Scripts\activate
```

**手动使用工具:**
```bash
# 激活环境后直接使用
mineru --help
python -m markitdown --help

# 或使用uv直接运行
uv run mineru --help
uv run python -m markitdown --help
```

### 使用流程
1. 在 mcp 社区查找并复制所需插件的 JSON 配置。
2. 将 JSON 配置添加到本服务的插件配置中。
3. 服务器自动加载插件并启动服务，生成对应的 SSE URL。
4. 客户端通过该 URL 地址，即可实时获取 mcp 服务推送的数据。

## 🔧 Voice CLI 配置说明

### 配置文件模板

**服务器配置 (server-config.yml)：**
- 单节点语音转录服务配置
- 支持 Whisper 模型管理
- 音频处理设置和并发控制
- 日志和守护进程配置

**集群配置 (cluster-config.yml)：**
- 集群节点配置，包含 gRPC 通信
- 节点 ID、端口和任务处理设置
- 集群元数据存储和心跳配置
- 支持多节点协同工作

**负载均衡器配置 (lb-config.yml)：**
- 负载均衡服务配置
- 健康检查间隔和超时设置

### 高级功能

**配置文件管理：**
```bash
# 查看生成的配置文件
cat server-config.yml
cat cluster-config.yml
cat lb-config.yml

# 编辑配置文件
vim server-config.yml
vim cluster-config.yml
vim lb-config.yml
```

## API 接口说明

### 1. MCP 服务状态检查接口

此接口用于检查指定 MCP 服务的状态，如果服务不存在，系统会自动启动对应的 MCP 服务。

```
POST http://localhost:8020/mcp/sse/check_status
Content-Type: application/json
```

请求参数：
```json
{
  "mcpId": "服务唯一标识符",
  "mcpJsonConfig": "MCP服务的JSON配置",
  "mcpType": "服务类型（如Persistent）"
}
```

参数说明：
- `mcpId`: 为 MCP 服务指定的唯一标识符，用于后续访问该服务
- `mcpJsonConfig`: MCP 插件的 JSON 配置，包含命令、参数和环境变量等
- `mcpType`: MCP 服务类型，如 "Persistent"（持久服务）;"OneShot"(短时服务)

示例：
```json
{
  "mcpId": "mysql-test-id",
  "mcpJsonConfig": "{\"mcpServers\": {\"mysql\": {\"command\": \"go-mcp-mysql\", \"args\": [\"--host\", \"192.168.1.12\", \"--user\", \"username\", \"--pass\", \"password\", \"--port\", \"3306\", \"--db\", \"database_name\"], \"env\": {}}}}",
  "mcpType": "Persistent"
}
```

### 2. SSE 协议连接接口

成功启动 MCP 服务后，可通过以下 URL 建立 SSE 连接，实时接收服务推送的数据：

```
http://localhost:8080/mcp/sse/proxy/{mcpId}/sse
```

参数说明：
- `{mcpId}`: 在状态检查接口中指定的 MCP 服务唯一标识符
- 请求header属性: x-mcp-json ,附加 mcp json配置
- 请求header属性: x-mcp-type ,附加 MCP 服务类型，如 "Persistent"（持久服务）;"OneShot"(短时服务)

示例：
```
http://localhost:8080/mcp/sse/proxy/mysql-test-id/sse
```

注意：此 URL 需要在支持 SSE 协议的客户端中打开，如浏览器或支持 SSE 的应用程序。

### 3. 向 MCP 服务发送消息接口

通过此接口向已启动的 MCP 服务发送消息：

```
POST http://localhost:8080/mcp/sse/proxy/{mcpId}/message
Content-Type: application/json
```

参数说明：
- `{mcpId}`: MCP 服务的唯一标识符
- 请求header属性: x-mcp-json ,附加 mcp json配置
- 请求header属性: x-mcp-type ,附加 MCP 服务类型，如 "Persistent"（持久服务）;"OneShot"(短时服务)

请求体示例：
```json
{
  "id": "消息ID",
  "method": "调用的方法",
  "params": {
    "messages": [
      {
        "role": "user",
        "content": "具体的指令内容"
      }
    ]
  }
}
```

示例（查询 MySQL 数据库中的所有表）：
```json
{
  "id": "msg_123",
  "method": "completions.create",
  "params": {
    "messages": [
      {
        "role": "user",
        "content": "查询当前数据库中的所有表"
      }
    ]
  }
}
```

---

## 环境设置

### 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 安装 VSCode 插件

- crates: Rust 包管理
- Even Better TOML: TOML 文件支持
- Better Comments: 优化注释显示
- Error Lens: 错误提示优化
- GitLens: Git 增强
- Github Copilot: 代码提示
- indent-rainbow: 缩进显示优化
- Prettier - Code formatter: 代码格式化
- REST client: REST API 调试
- rust-analyzer: Rust 语言支持
- Rust Test lens: Rust 测试支持
- Rust Test Explorer: Rust 测试概览
- TODO Highlight: TODO 高亮
- vscode-icons: 图标优化
- YAML: YAML 文件支持

### 安装 cargo generate

cargo generate 是一个用于生成项目模板的工具。它可以使用已有的 github repo 作为模版生成新的项目。

```bash
cargo install cargo-generate
```

在我们的课程中，新的项目会使用 `tyr-rust-bootcamp/template` 模版生成基本的代码：

```bash
cargo generate tyr-rust-bootcamp/template
```

### 安装 pre-commit

pre-commit 是一个代码检查工具，可以在提交代码前进行代码检查。

```bash
pipx install pre-commit
```

安装成功后运行 `pre-commit install` 即可。

### 安装 Cargo deny

Cargo deny 是一个 Cargo 插件，可以用于检查依赖的安全性。

```bash
cargo install --locked cargo-deny
```

### 安装 typos

typos 是一个拼写检查工具。

```bash
cargo install typos-cli
```

### 安装 git cliff

git cliff 是一个生成 changelog 的工具。

```bash
cargo install git-cliff
```

### 安装 cargo nextest

cargo nextest 是一个 Rust 增强测试工具。

```bash
cargo install cargo-nextest --locked
```

## 🔧 Document Parser 故障排除

### 常见问题

#### 1. 虚拟环境问题

**问题：虚拟环境创建失败**
```bash
# 检查当前目录权限
ls -la  # Linux/macOS
dir     # Windows

# 解决方案
chmod 755 .              # Linux/macOS 修改权限
rm -rf ./venv            # 删除损坏的虚拟环境
document-parser uv-init  # 重新初始化
```

## 🔧 Voice CLI 故障排除

### 配置文件初始化问题

**问题：配置文件已存在**
```bash
# 解决方案：使用 --force 参数覆盖
voice-cli server init --force
voice-cli cluster init --force
voice-cli lb init --force
```

**问题：配置文件路径问题**
```bash
# 解决方案：指定正确的配置文件路径
voice-cli server init --config ./my-server-config.yml
voice-cli cluster init --config ./my-cluster-config.yml
voice-cli lb init --config ./my-lb-config.yml
```

**问题：虚拟环境激活失败**
```bash
# Linux/macOS
source ./venv/bin/activate

# Windows PowerShell (如果执行策略限制)
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
.\venv\Scripts\Activate.ps1

# Windows CMD
.\venv\Scripts\activate
```

#### 2. 依赖安装问题

**问题：UV工具未安装**
```bash
# 官方安装脚本
curl -LsSf https://astral.sh/uv/install.sh | sh

# 或使用包管理器
brew install uv          # macOS
pip install uv           # 通用方法
```

**问题：网络连接问题**
```bash
# 使用国内镜像源
uv pip install -i https://pypi.tuna.tsinghua.edu.cn/simple/ mineru[core]

# 设置代理（如果需要）
export HTTP_PROXY=http://proxy:port
export HTTPS_PROXY=http://proxy:port
```

#### 3. 诊断命令

```bash
# 完整环境检查
document-parser check

# 详细故障排除指南
document-parser troubleshoot

# 手动验证
uv --version
./venv/bin/python --version    # Linux/macOS
.\venv\Scripts\python --version # Windows
```

### 环境要求

- **Python**: 3.8+ (推荐 3.11+)
- **操作系统**: Linux, macOS, Windows
- **磁盘空间**: 至少 500MB 可用空间
- **网络**: 需要访问 PyPI (或配置镜像源)
- **CUDA** (可选): 用于GPU加速，支持CUDA 11.8+

### Voice CLI 环境要求

- **Rust**: 1.70+ (推荐 1.75+)
- **操作系统**: Linux, macOS, Windows
- **磁盘空间**: 至少 200MB 可用空间
- **网络**: 需要访问 crates.io (或配置镜像源)

### 目录结构

```
document-parser/
├── venv/                    # 虚拟环境 (自动创建)
│   ├── bin/                 # Linux/macOS 可执行文件
│   ├── Scripts/             # Windows 可执行文件
│   └── lib/                 # Python包
├── logs/                    # 日志文件
├── config.yml               # 配置文件
└── src/                     # 源代码

voice-cli/
├── server-config.yml        # 服务器配置文件 (init 生成)
├── cluster-config.yml       # 集群配置文件 (init 生成)
├── lb-config.yml            # 负载均衡器配置文件 (init 生成)
├── logs/                    # 日志文件目录
├── models/                  # Whisper 模型存储目录
├── cluster_metadata/        # 集群元数据存储
├── *.pid                    # 进程 ID 文件
└── src/                     # 源代码
```

### 获取帮助

如果遇到问题：
1. 运行 `document-parser troubleshoot` 查看详细指南
2. 检查 `logs/` 目录中的日志文件
3. 确保在正确的项目目录中运行命令
4. 尝试在新目录中重新初始化环境

### Voice CLI 获取帮助

如果遇到问题：
1. 运行 `voice-cli --help` 查看所有可用命令
2. 运行 `voice-cli <command> --help` 查看特定命令的帮助
3. 检查生成的配置文件是否正确
