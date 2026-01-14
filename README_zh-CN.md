# MCP-Proxy Workspace

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP-Proxy 工作空间

一个基于 Rust 的综合工作空间，实现了 MCP (Model Context Protocol) 代理系统，包含文档解析、语音转录和协议转换等多个服务。

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

## 工作空间成员

| Crates | 版本 | 描述 |
|--------|------|------|
| **mcp-common** | 0.1.5 | MCP 代理组件的共享类型和工具 |
| **mcp-sse-proxy** | 0.1.5 | 基于 rmcp 0.10 的 SSE (Server-Sent Events) 代理实现 |
| **mcp-streamable-proxy** | 0.1.5 | 基于 rmcp 0.12 的 Streamable HTTP 代理实现 |
| **mcp-stdio-proxy** | 0.1.18 | 主 MCP 代理服务器，带 CLI 工具用于协议转换 |
| **document-parser** | 0.1.0 | 高性能多格式文档解析服务 |
| **voice-cli** | 0.1.0 | 基于 Whisper 模型的语音转文字 HTTP 服务 |
| **oss-client** | 0.1.0 | 轻量级阿里云 OSS 客户端库 |
| **fastembed** | 0.1.0 | 使用 FastEmbed 的文本嵌入 HTTP 服务 |

## 快速开始

### 环境要求

- **Rust**: 1.70 或更高版本（推荐 1.75+）
- **Python**: 3.8+（用于 document-parser 和 voice-cli TTS）
- **uv**: Python 包管理器（通过 `curl -LsSf https://astral.sh/uv/install.sh | sh` 安装）

### 安装

```bash
# 克隆仓库
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy

# 构建所有工作空间成员
cargo build --release

# 或构建特定 crate
cargo build -p mcp-proxy
cargo build -p document-parser
cargo build -p voice-cli
```

### MCP 代理 (mcp-stdio-proxy)

主代理服务，将 SSE/Streamable HTTP 转换为 stdio 协议。

```bash
# 从源码安装
cargo install --path ./mcp-proxy

# 启动代理服务器
mcp-proxy

# 将远程 MCP 服务转换为 stdio
mcp-proxy convert https://example.com/mcp/sse

# 检查服务状态
mcp-proxy check https://example.com/mcp/sse

# 检测协议类型
mcp-proxy detect https://example.com/mcp
```

**详细文档:** [mcp-proxy/README_zh-CN.md](./mcp-proxy/README_zh-CN.md)

### 文档解析器

支持 PDF、Word、Excel 和 PowerPoint 的高性能文档解析服务。

```bash
cd document-parser

# 初始化 Python 环境（首次使用）
document-parser uv-init

# 检查环境状态
document-parser check

# 启动 HTTP 服务器
document-parser server
```

**详细文档:** [document-parser/README_zh-CN.md](./document-parser/README_zh-CN.md)

### 语音 CLI

基于 Whisper 模型的语音转文字 HTTP 服务。

```bash
cd voice-cli

# 初始化服务器配置
voice-cli server init

# 运行语音服务器
voice-cli server run

# 列出 Whisper 模型
voice-cli model list

# 下载模型
voice-cli model download tiny
```

**详细文档:** [voice-cli/README_zh-CN.md](./voice-cli/README_zh-CN.md)

## 架构

### 核心服务

#### 1. MCP 代理系统

- **mcp-common**: 共享配置类型和工具
- **mcp-sse-proxy**: SSE 协议支持 (rmcp 0.10)
- **mcp-streamable-proxy**: Streamable HTTP 协议支持 (rmcp 0.12)
- **mcp-stdio-proxy**: 用于协议转换的主 CLI 工具

**特性:**
- 多协议支持：SSE、Streamable HTTP、stdio
- 动态插件加载
- 协议自动检测和转换
- OpenTelemetry 集成，支持 OTLP
- 后台健康检查

#### 2. 文档解析器

**特性:**
- 多格式支持：PDF (MinerU)、Word/Excel/PowerPoint (MarkItDown)
- GPU 加速，通过 CUDA/sglang（可选）
- 使用 uv 进行 Python 环境管理
- HTTP API，带 OpenAPI 文档
- OSS 云存储集成

#### 3. 语音 CLI

**特性:**
- Whisper 模型集成（tiny/base/small/medium/large）
- 多格式音频支持（MP3、WAV、FLAC、M4A 等）
- 基于 Apalis 的异步任务队列，带 SQLite 持久化
- FFmpeg 集成，用于元数据提取
- **TTS 服务（TODO - 当前存在问题）**

#### 4. 工具库

- **oss-client**: 阿里云 OSS 客户端，统一接口
- **fastembed**: 使用 FastEmbed 的文本嵌入 HTTP 服务

## 开发

### 构建命令

```bash
# 构建所有工作空间 crates
cargo build

# 构建特定 crate
cargo build -p mcp-proxy

# 以 release 模式构建
cargo build --release

# 运行所有 crates 的测试
cargo test

# 运行特定 crate 的测试
cargo test -p mcp-proxy

# 运行 clippy 进行代码检查
cargo clippy --all-targets --all-features

# 格式化代码
cargo fmt
```

### 跨平台构建 (Docker)

```bash
# 为 Linux x86_64 构建 document-parser
make build-document-parser-x86_64

# 为 Linux ARM64 构建 document-parser
make build-document-parser-arm64

# 为 Linux x86_64 构建 voice-cli
make build-voice-cli-x86_64

# 为 x86_64 构建所有组件
make build-all-x86_64

# 构建 Docker 运行时镜像
make build-image

# 运行 Docker 容器
make run
```

### 代码风格

- 行长度：100 字符
- 4 空格缩进（不使用制表符）
- 使用 `dashmap` 替代 `Arc<RwLock<HashMap>>` 进行并发哈希映射
- 遵循 KISS 和 SOLID 原则
- 使用 `anyhow::Context` 实现"尽快失败"错误处理

## 文档

- [CLAUDE.md](./CLAUDE.md) - 贡献者开发指南
- [mcp-proxy/README_zh-CN.md](./mcp-proxy/README_zh-CN.md) - MCP 代理文档
- [document-parser/README_zh-CN.md](./document-parser/README_zh-CN.md) - 文档解析器文档
- [voice-cli/README_zh-CN.md](./voice-cli/README_zh-CN.md) - 语音 CLI 文档
- [oss-client/README_zh-CN.md](./oss-client/README_zh-CN.md) - OSS 客户端文档

## 许可证

本项目采用 MIT OR Apache-2.0 双许可证。

## 贡献

欢迎贡献！请随时提交问题和拉取请求。

- **GitHub 仓库**: https://github.com/nuwax-ai/mcp-proxy
- **问题跟踪**: https://github.com/nuwax-ai/mcp-proxy/issues
- **讨论区**: https://github.com/nuwax-ai/mcp-proxy/discussions

## 相关资源

- [MCP 官方文档](https://modelcontextprotocol.io/)
- [rmcp - Rust MCP 实现](https://crates.io/crates/rmcp)
- [MCP 服务器列表](https://github.com/modelcontextprotocol/servers)
