# Docker 配置说明

本目录包含 mcp-proxy 项目的所有 Docker 相关配置文件，与线上环境保持依赖一致。

## 文件说明

### Dockerfile.mcp-proxy
mcp-proxy 的 Docker 构建文件，采用多阶段构建：

**构建阶段：**
- 基础镜像：`rust:1.92`
- 设置时区：`Asia/Shanghai`
- 构建命令：`cargo build --release --bin mcp-proxy`

**运行阶段：**
- 基础镜像：`rust:1.92`
- 包含完整的运行时环境（与线上环境一致）
- 支持 Node.js 22.x、Python 3、Deno、Go 1.24.3

### Dockerfile.document-parser
document-parser 的 Docker 构建文件，采用多阶段构建：

**构建阶段：**
- 基础镜像：`rust:1.92`
- 构建命令：`cargo build --release`

**运行/导出阶段：**
- 使用 `scratch` 基础镜像（最小化镜像）
- 支持目标：`runtime`（document-parser 运行时）/ `export`（导出二进制）

### Dockerfile.voice-cli
voice-cli 的 Docker 构建文件（独立）。编译期需 sherpa-onnx 预编译 C 库：无 `SHERPA_ONNX_ARCHIVE_DIR` 缓存时默认联网下载；github 阻断时先 `bash docker/fetch-sherpa.sh amd64` 预下载到 `docker/sherpa-cache/`。

- 基础镜像：`rust:1.92`
- 构建命令：`cargo build --release -p voice-cli`（可选 `--build-arg VOICE_FEATURES=cuda`）
- 导出目标：`export`（只导出二进制到 `dist/`，不在容器运行）

### Dockerfile.fastembed
fastembed 文本嵌入服务的 Docker 构建文件（独立）。用法：`make build-fastembed-x86_64`。

### config.yml
mcp-proxy 的默认配置文件。

### docker-compose.yml
Docker Compose 配置文件，用于快速启动 mcp-proxy 服务。

### .npmrc
npm 国内镜像源配置（淘宝镜像），用于 Node.js 包管理：
```ini
registry=https://registry.npmmirror.com/
```

### pip.conf
pip 国内镜像源配置（清华大学镜像），用于 Python 包管理：
```ini
[global]
index-url = https://pypi.tuna.tsinghua.edu.cn/simple
trusted-host = pypi.tuna.tsinghua.edu.cn
```

## 运行时环境

### mcp-proxy 容器

| 环境 | 版本 | 用途 |
|------|------|------|
| Rust | 1.92 | 基础运行时 |
| Node.js | 22.x | run_code 功能执行 Node.js 代码 |
| Python | 3.x + uv | run_code 功能执行 Python 代码 |
| Deno | 最新版 | run_code 功能执行 TypeScript/JavaScript 代码 |
| Go | 1.24.3 | run_code 功能执行 Go 代码 |

### 额外工具

| 工具 | 用途 |
|------|------|
| ffmpeg | 音视频处理 |
| vim | 文本编辑 |
| net-tools | 网络工具 |
| telnet | 网络调试 |
| wget | 文件下载 |

### document-parser / voice-cli / fastembed 容器

使用 `scratch` 基础镜像仅导出二进制（`--target export`），**不在容器内运行**；二进制部署到目标机，运行时依赖（如 ffmpeg）由目标机提供。注：voice-cli 编译期还需 sherpa-onnx 预编译 C 库（见上 `Dockerfile.voice-cli` 段），与另两者不同。

## 国内镜像源配置

容器内已配置以下国内镜像源：

| 工具 | 镜像源 | 配置位置 |
|------|--------|----------|
| npm | 淘宝镜像 | `/root/.npmrc` |
| pip | 清华大学镜像 | `/etc/pip.conf` |
| uv | 阿里云镜像 | `UV_INDEX_URL` 环境变量 |

## Go MCP 工具

mcp-proxy 容器内预装了 `go-mcp-mysql` 工具用于测试：
```bash
go install -v github.com/Zhwt/go-mcp-mysql@latest
```

## 使用方法

### 构建 mcp-proxy 镜像

```bash
# 使用 docker build
docker build -f docker/Dockerfile.mcp-proxy -t mcp-proxy:latest ..

# 或使用 Make 命令
make build-image

# 构建 ARM64 镜像
docker build -f docker/Dockerfile.mcp-proxy --platform linux/arm64 -t mcp-proxy:latest ..
```

### 构建 document-parser 镜像

```bash
# 使用 docker build
docker build -f docker/Dockerfile.document-parser --target runtime -t document-parser:latest ..

# 或使用 Make 命令
make build-image-document-parser

# 构建 voice-cli（导出 linux 二进制到 dist/，github 阻断时先 bash docker/fetch-sherpa.sh amd64）
make build-voice-cli-x86_64
```

### 使用 docker-compose（推荐）

```bash
# 启动服务
cd docker && docker-compose up

# 后台启动
cd docker && docker-compose up -d

# 查看日志
cd docker && docker-compose logs -f

# 停止服务
cd docker && docker-compose down
```

### 使用 Make 命令

```bash
# mcp-proxy 相关
make build-mcp-proxy-x86_64         # 构建 mcp-proxy x86_64 版本
make build-image                    # 构建 mcp-proxy Docker 镜像
make run-compose                    # 使用 docker-compose 启动

# document-parser 相关
make build-document-parser-x86_64   # 构建 document-parser x86_64 版本
make build-image-document-parser    # 构建 document-parser Docker 镜像

# voice-cli 相关
make build-voice-cli-x86_64         # 构建 voice-cli x86_64 版本
```

## 环境变量

### mcp-proxy 容器

| 环境变量 | 说明 | 默认值 |
|----------|------|--------|
| RUST_LOG | 日志级别 | info |
| TZ | 时区 | Asia/Shanghai |
| UV_INDEX_URL | uv 镜像源 | https://mirrors.aliyun.com/pypi/simple |

## 挂载目录

### mcp-proxy 容器

- `./config.yml` - 配置文件（只读）
- `./logs` - 日志目录（持久化）

## 健康检查

mcp-proxy 容器包含健康检查，定期检查 `/health` 端点：
- 检查间隔：30 秒
- 超时时间：10 秒
- 重试次数：3 次
- 启动等待：5 秒

## 线上环境一致性

本目录的 Dockerfile 与线上环境保持依赖一致：
- `Dockerfile.mcp-proxy` 对应 `/Volumes/soddygo/git_work/build-agent-docker/build_config/mcp_proxy/Dockerfile`

## 目录结构

```
docker/
├── Dockerfile.mcp-proxy           # mcp-proxy Docker 构建文件
├── Dockerfile.document-parser     # document-parser Docker 构建文件
├── Dockerfile.voice-cli           # voice-cli Docker 构建文件（独立）
├── Dockerfile.fastembed           # fastembed Docker 构建文件（独立）
├── fetch-sherpa.sh                # sherpa-onnx C 库预下载（gh-proxy，阻断免疫）
├── sherpa-cache/                  # 预下载的 sherpa tar.bz2 缓存（.gitignore 拦截不入库）
├── config.yml                     # mcp-proxy 默认配置
├── docker-compose.yml             # Docker Compose 配置
├── .npmrc                         # npm 国内镜像源配置
├── pip.conf                       # pip 国内镜像源配置
└── README.md                      # 本文档
```
