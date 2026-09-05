# Document Parser 安装指南

多格式文档解析服务（PDF / Word / Excel / PowerPoint / Markdown 等 → 结构化 Markdown），Rust HTTP 服务 + Python 解析引擎（MinerU / MarkItDown）。支持 **Linux / macOS / Windows** 三平台。

- 架构：单个 Rust 二进制（HTTP API + 任务队列 + 存储），通过子进程调用 `./venv` 内的 Python 解析引擎；Python 环境由服务自动管理（uv）
- 上传后端二选一：阿里云 OSS **或** 自建系统文件接口（nuwax 风格，见 [CUSTOM_UPLOAD_API.md](CUSTOM_UPLOAD_API.md)）

## 1. 系统要求

| 项目 | Linux | macOS | Windows |
|------|-------|-------|---------|
| 操作系统 | Ubuntu 18.04+ 等 | **14.0+**（MinerU 要求） | Windows 10 / 11 |
| 硬件 | 8GB+ RAM，5GB+ 磁盘 | 同左（Apple Silicon 原生支持） | 同左 |
| Python | 3.10+（uv 自动创建 venv，无需系统预装） | 同左 | 同左 |
| Rust 工具链 | 仅源码构建需要 | 仅源码构建需要 | 仅源码构建需要 |
| GPU（可选） | CUDA 加速 PDF 解析 | 不适用（MPS/CPU） | 不适用（CPU） |

> 国内网络环境：安装器会自动检测并使用阿里云 PyPI 镜像与国内模型源，无需手工配置。

## 2. 安装服务二进制

### 方式一：预编译二进制（推荐，无需 Rust）

各平台安装脚本（GitHub Releases）：

```bash
# Linux / macOS
curl -proto '=https' -tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.sh | sh

# Windows PowerShell
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.ps1 | iex
```

或从 [GitHub Releases](https://github.com/nuwax-ai/mcp-proxy/releases) 手动下载对应平台产物（Linux x86_64 / ARM64、macOS Intel / Apple Silicon、Windows x86_64）。

### 方式二：源码构建

```bash
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy
cargo build --release -p document-parser
# 产物：target/release/document-parser
```

> Docker 交叉编译（Linux x86_64 / ARM64）：`make build-document-parser-x86_64` 等，见仓库 `Makefile`。

## 3. 初始化 Python 解析引擎

进入服务的工作目录（二进制所在目录，或任意空目录），执行：

```bash
document-parser uv-init
```

该命令自动完成：安装 [uv](https://docs.astral.sh/uv/) → 创建 `./venv` → 安装 MinerU（按 CUDA 环境自动选 `mineru[all]` 或 `mineru[core]`）与 MarkItDown。**需要联网**，大包下载耗时数分钟。

也可以跳过此步直接启动服务：首次启动检测到依赖缺失时会在后台自动安装（服务正常监听，安装完成前解析任务排队等待）。

验证环境：

```bash
document-parser check
```

## 4. 配置

服务按顺序读取配置：代码默认值 → `config.yml`（`--config` 指定，默认在当前目录）→ 环境变量（含 `.document-parser.env` 文件，该文件已被 gitignore，适合放凭证）→ 命令行参数。

首次启动会在当前目录生成默认 `config.yml`（完整示例见 [deploy/config/config.example.yml](deploy/config/config.example.yml)）。关键配置：

**上传后端（二选一）**——默认走阿里云 OSS：

```yaml
storage:
  oss:
    endpoint: "oss-rg-china-mainland.aliyuncs.com"
    public_bucket: "your-public-bucket"
    private_bucket: "your-private-bucket"
    access_key_id: "${OSS_ACCESS_KEY_ID}"      # 从 .document-parser.env 或环境变量读
    access_key_secret: "${OSS_ACCESS_KEY_SECRET}"
```

或改用自建系统上传接口（配置后所有请求默认走该后端；也可不配全局，按请求传 `upload_*` 参数）：

```yaml
storage:
  custom_upload:
    base_url: "https://your-system.example.com"   # 留空 = 不启用（走 OSS）
    api_key: ""                                   # Bearer API Key
    path: "/api/v1/file/upload"                   # nuwax 契约默认值
```

监听地址与端口：

```yaml
server:
  host: "0.0.0.0"
  port: 8077
```

## 5. 启动与验证

```bash
# 前台启动（端口可由 --port 覆盖）
document-parser server

# 健康检查 / API 文档
curl http://localhost:8077/health
# Swagger UI: http://localhost:8077/api/docs
```

快速验证解析链路（同步接口，小文件）：

```bash
curl -X POST "http://localhost:8077/api/v1/documents/parse-sync" -F "file=@test.md"
```

遇到环境问题（Python 缺失、模型下载失败等）运行诊断：

```bash
document-parser troubleshoot
```

## 6. 设为系统服务（开机自启）

内置服务安装命令（复用 deploy-installer）：

```bash
document-parser service install      # 安装并启动（以当前工作目录为服务目录）
document-parser service status
document-parser service restart
document-parser service uninstall
```

| 平台 | 服务管理器 | 说明 |
|------|-----------|------|
| Linux | systemd | 原生支持；用户级或系统级单元，详见 [SYSTEMD_SETUP_GUIDE.md](SYSTEMD_SETUP_GUIDE.md) |
| macOS | launchd | 原生支持；生成 plist 并加载 |
| Windows | —（无内置） | 用 [NSSM](https://nssm.cc/) 注册系统服务：`nssm install DocumentParser "C:\path\document-parser.exe" "server"`；或用任务计划程序设置开机启动 |

> 服务的工作目录决定 `config.yml` / `venv` / 数据目录的位置——安装服务前先在该目录完成第 3、4 步。

## 7. 平台特定注意事项

**macOS**
- 必须 macOS 14.0+（MinerU 的要求）；Apple Silicon 原生运行，无需 Rosetta
- GPU 加速不适用，PDF 解析走 CPU（M 系列芯片性能足够）
- launchd 无 `EnvironmentFile` 概念，凭证放工作目录的 `.document-parser.env`（服务启动时自动加载）

**Linux**
- 可选 CUDA 加速 PDF 解析，环境配置见 [CUDA_SETUP_GUIDE.md](CUDA_SETUP_GUIDE.md)；无 GPU 自动回退 CPU
- 生产部署样例（含 systemd、目录布局、踩坑记录）见 [deploy/PITFALLS.md](deploy/PITFALLS.md)

**Windows**
- 推荐 Windows 10/11，8GB+ 内存
- 工作目录路径不要超过 260 字符（虚拟环境路径长度限制，服务启动时会检查并提示）
- uv 在 Windows 优先走 PowerShell 安装脚本；若 PowerShell 执行策略受限，先手动安装 uv：`winget install astral-sh.uv`
- 常驻服务用 NSSM 或任务计划程序（见上节）

## 8. 目录布局

运行后工作目录下产生：

```
./
├── document-parser      # 二进制（预编译方式）
├── config.yml           # 配置（首次启动生成）
├── .document-parser.env # 凭证（可选，gitignored）
├── venv/                # Python 解析引擎环境（uv 管理）
├── data/document_parser # sled 任务数据库
├── logs/                # 按天轮转日志
└── temp/                # 解析临时文件（任务过期后自动清理）
```

## 9. 下一步

- 接口契约与参数：Swagger UI（`/api/docs`）或 [README_zh-CN.md](README_zh-CN.md)
- 自建上传后端对接：[CUSTOM_UPLOAD_API.md](CUSTOM_UPLOAD_API.md)
- 故障排查：[TROUBLESHOOTING.md](TROUBLESHOOTING.md)
