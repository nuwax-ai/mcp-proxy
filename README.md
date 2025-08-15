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
实现了一个 mcp 代理服务，用户可以通过 SSE（Server-Sent Events）协议，配置我们提供的 URL 地址，远程使用服务器提供的 mcp 功能。

**主要功能:**
- 支持通过 SSE 协议与客户端通信，实时推送数据。
- 支持动态添加 mcp 插件：只需在 mcp 社区查找所需插件，复制对应的 JSON 配置，粘贴到本服务的配置中，即可自动加载并启用插件。
- 每个插件配置完成后，服务器会自动启动对应的 mcp 服务，并生成可供访问的 SSE 协议 URL 地址。
- 用户可通过该 URL 地址，直接使用远程服务器的 mcp 能力。

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
```

### 获取帮助

如果遇到问题：
1. 运行 `document-parser troubleshoot` 查看详细指南
2. 检查 `logs/` 目录中的日志文件
3. 确保在正确的项目目录中运行命令
4. 尝试在新目录中重新初始化环境
