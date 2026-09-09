# Document Parser 安装指南

多格式文档解析服务（PDF / Word / Excel / PowerPoint / Markdown 等 → 结构化 Markdown），Rust HTTP 服务 + Python 解析引擎（MinerU / MarkItDown）。支持 **Linux / macOS / Windows** 三平台。

- 架构：单个 Rust 二进制（HTTP API + 任务队列 + 存储），通过子进程调用 `./venv` 内的 Python 解析引擎；Python 环境由服务自动管理（uv）
- 上传后端二选一：阿里云 OSS **或** 自建系统文件接口（nuwax 风格，见 [CUSTOM_UPLOAD_API.md](CUSTOM_UPLOAD_API.md)）

## 1. 系统要求

| 项目 | Linux | macOS | Windows |
|------|-------|-------|---------|
| 操作系统 | Ubuntu 22.04+（glibc ≥ 2.35）等 | **14.0+**（MinerU 要求） | Windows 10 / 11 |
| 硬件 | 8GB+ RAM，5GB+ 磁盘 | 同左（Apple Silicon 原生支持） | 同左 |
| 系统库 | X11/GL 基础库（无桌面服务器需预装）：Debian/Ubuntu `apt install libxcb1 libxkbcommon-x11-0 libgl1 libglib2.0-0`；RHEL 系 `dnf install libxcb libxkbcommon libXext libXrender mesa-libGL glib2` | 无需 | 无需 |
| Python | 3.10+（uv 自动创建 venv，无需系统预装） | 同左 | 同左 |
| Node.js | 仅 deploy-installer 方式需要（18+） | 同左 | —（该方式不支持 Windows） |
| Rust 工具链 | 仅 `cargo install` 方式需要 | 仅 `cargo install` 方式需要 | 仅 `cargo install` 方式需要 |
| GPU（可选） | CUDA 加速 PDF 解析 | 不适用（MPS/CPU） | 不适用（CPU） |

> 国内网络环境：安装器会自动检测并使用阿里云 PyPI 镜像与国内模型源，无需手工配置。

## 2. 安装服务二进制

三种方式按推荐顺序排列。**deploy-installer 最省事**（自动装 venv、注册服务、等健康检查），**cargo install 不需要获取源码**，二者都不接触本仓库源码。

### 方式一：deploy-installer（推荐，macOS / Linux）

统一部署 CLI，以 npm 包发布、**二进制内置在包内**（无需从 GitHub Releases 下载）：

```bash
# 需要 Node.js 18+（macOS: brew install node）
npm install -g nuwax-deploy-installer
# 环境自检（平台、二进制、磁盘空间、GUI 会话）
deploy-installer doctor

# 一键安装 document-parser：复制二进制 → 下载预编译 venv（约 300MB）
# → 写配置 → 注册系统服务（launchd/systemd）→ 等待 /health 就绪
deploy-installer document-parser install

# 服务管理
deploy-installer document-parser service status
```

> 🇨🇳 **国内网络建议先配置 npm 镜像**（包体积约 80MB，直连 npmjs 仅 ~200KB/s，
> npmmirror 实测快 40 倍以上）：
>
> ```bash
> npm config set registry https://registry.npmmirror.com
> ```
>
> 注意 npmmirror 同步有约 10–60 分钟延迟——刚发布的最新 beta 可能暂未同步，
> 急用可临时直连：`npm install -g nuwax-deploy-installer@beta --registry https://registry.npmjs.org`。
> 另外普通用户全局安装需要 sudo——**sudo 不会读取用户级 `~/.npmrc` 的镜像配置**，
> 镜像对 sudo 安装生效需内联传参：
>
> ```bash
> sudo npm install -g nuwax-deploy-installer@beta --registry https://registry.npmmirror.com
> ```
>
> Linux systemd 机器还需非交互 sudo：在 sudoers 配置受限 NOPASSWD（推荐做法）
> `用户名 ALL=(root) NOPASSWD: /usr/bin/systemctl, /usr/bin/journalctl, /usr/bin/install, /usr/bin/mkdir, /usr/bin/rm`
> （systemctl/journalctl 管理服务 + install/mkdir/rm 写删 unit 文件——226 实测缺后三枚会在 unit 落盘时挂）。

document-parser 需要上传后端凭证（**OSS 或自定义上传后端二选一**），安装前先导出（也可装完后写入 `~/document-parser/.document-parser.env`）：

```bash
# 方案 A：阿里云 OSS（云端部署）
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret

# 方案 B：自建系统上传接口（私有部署，nuwax 风格——契约见 CUSTOM_UPLOAD_API.md）
export DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://your-system.example.com
export DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=你的APIKey
```

纯内网环境（无法访问 OSS 下载源）可用本地 venv 包离线安装：

```bash
deploy-installer document-parser install --venv-file /path/to/venv-macos-arm64-x.y.z.tar.gz
```

- 默认安装目录 `~/document-parser`，端口 8087；**不要**装在 Documents / Desktop / iCloud 目录（macOS 服务权限限制）
- macOS 上安装命令需要当前用户在本机图形界面登录（纯 SSH 场景见 [mac-mini-quickstart.md](../deploy-installer/doc/mac-mini-quickstart.md)）
- 支持平台：macOS Apple Silicon、Linux x86_64（voice-cli 三档自动检测：NVIDIA CUDA 包 / AMD·Intel Vulkan 包 / CPU）、Windows x64（document-parser；voice-cli 视构建情况）——见 [MAINTAINER.md](../deploy-installer/doc/MAINTAINER.md)
- 详细步骤（含 voice-cli 组合部署、SSH 场景）见 [mac-mini-quickstart.md](../deploy-installer/doc/mac-mini-quickstart.md)

### 方式二：cargo install（全平台，含 Windows）

不获取源码，直接从 git 仓库编译安装到 `~/.cargo/bin`（需要 [Rust 工具链](https://rustup.rs)，仅此一次性依赖）：

```bash
cargo install --git https://github.com/nuwax-ai/mcp-proxy document-parser --locked
document-parser --version
```

> ⚠️ 包名说明：crates.io 上的 `document-parser` 是**无关的第三方包**，不要 `cargo install document-parser`；本服务只能用 `--git` 方式安装。

Windows 用户主路径即此方式（Rust MSVC 工具链 + 本命令），随后同样执行第 3 步初始化 Python 引擎。

> ⚠️ **默认安装分支提示**：`cargo install --git` 不带 `--tag/--branch` 时安装 **main** 分支；新功能先发布在发布 tag 上，稳定用户可显式指定，如 `cargo install --git https://github.com/nuwax-ai/mcp-proxy --tag deploy-v0.2.9 document-parser --locked`。

**Windows 实测注意事项**（Win11 + MSVC 验证于 0.2.9 系列）：

1. 编译期 `utoipa-swagger-ui` 需从 GitHub 下载 swagger-ui zip，若系统 curl 报 SSL 错误（exit 35），提前下载 [v5.17.14.zip](https://github.com/swagger-api/swagger-ui/archive/refs/tags/v5.17.14.zip) 并设置 `SWAGGER_UI_DOWNLOAD_URL=file:///C:/path/to/swagger-ui-v5.17.14.zip` 后重新安装
2. `uv-init` 前先把 [config.example.yml](deploy/config/config.example.yml) 放到工作目录并重命名为 `config.yml`——空目录首次运行会生成 OSS 字段为空的默认配置，启动即被 Fail-Fast 校验拒绝
3. 系统需有 `python`（3.10+）；若遇 `os error 448 无法遍历…不受信任的装入点`（uv 自管 Python 的 junction 被 Windows 安全补丁拒绝），设置环境变量 `UV_PYTHON_INSTALL_DIR` 指向一个新的空目录后重跑 `uv-init`，uv 会改用系统 Python

### 方式三：GitHub Releases 手动下载

从 [Releases](https://github.com/nuwax-ai/mcp-proxy/releases) 下载对应平台产物（Linux x86_64 / ARM64、macOS Intel / Apple Silicon、Windows x86_64），解压到目标目录即可。适合离线环境或不想装 Node / Rust 的场景。

## 3. 初始化 Python 解析引擎

> **deploy-installer 方式（方式一）已自动下载预编译 venv，跳过本节**。以下适用于 cargo install / Releases 方式。

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

> **deploy-installer 方式（方式一）在 install 时已注册并启动服务**，直接用 `deploy-installer document-parser service status/restart` 管理即可，跳过本节。

其余安装方式使用内置服务安装命令（复用 deploy-installer 的服务管理能力）：

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
├── document-parser      # 二进制（cargo install 后为 ~/.cargo/bin/document-parser）
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
