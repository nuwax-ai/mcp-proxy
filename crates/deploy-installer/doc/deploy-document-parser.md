# document-parser 部署指南

多格式文档解析服务（PDF / Word / Excel / PowerPoint / Markdown 等 → 结构化 Markdown）。Rust HTTP 服务 + Python 解析引擎（MinerU / MarkItDown），通过统一部署 CLI **deploy-installer**（npm 包 `nuwax-deploy-installer`）一键安装：复制二进制 → 准备 Python 环境 → 写配置 → 注册系统服务（launchd / systemd / 任务计划程序）→ 等待健康检查通过。

> voice-cli（语音转写/TTS）的部署见姊妹篇 [deploy-voice-cli.md](./deploy-voice-cli.md)。

## 1. 环境要求

| 项目 | Linux | macOS | Windows |
|------|-------|-------|---------|
| 操作系统 | Ubuntu 22.04+（glibc ≥ 2.35）等 | 14.0+（MinerU 要求） | Windows 10 / 11 |
| 硬件 | 8GB+ RAM，磁盘 5GB+ | 同左（Apple Silicon 原生） | 同左 |
| 系统库 | 无桌面服务器需预装 X11/GL 基础库（见 §7.2） | 无需 | 无需 |
| Node.js | 18+（deploy-installer 方式必需） | 同左 | 同左 |
| GPU（可选） | NVIDIA CUDA 加速 PDF 解析（见 §6） | 不适用（CPU） | 不适用（CPU） |

## 2. 安装 deploy-installer

```bash
npm install -g nuwax-deploy-installer
```

> 🇨🇳 国内网络建议先配 npm 镜像（包约 80MB，直连 npmjs 很慢，npmmirror 实测快 40 倍以上）：
>
> ```bash
> npm config set registry https://registry.npmmirror.com
> ```
>
> 注意：普通用户全局安装需要 sudo，而 **sudo 不读用户级 `~/.npmrc`**——镜像要对 sudo 安装生效需内联传参：
>
> ```bash
> sudo npm install -g nuwax-deploy-installer --registry https://registry.npmmirror.com
> ```
>
> npmmirror 同步有 10–60 分钟延迟，刚发布的 beta 可能未同步，急用可临时直连 npmjs。
>
> ⚠️ **镜像的 latest 元数据也可能滞后数天**（实测曾把 `npm install -g nuwax-deploy-installer` 装到数周前的旧 stable）——装完务必 `deploy-installer --version` 核对；版本旧就显式带版本号安装，或直连官方源：
>
> ```bash
> sudo npm install -g nuwax-deploy-installer@0.2.26 --registry https://registry.npmjs.org
> ```

装完自检环境（平台、二进制完整性、磁盘、系统库、上传后端配置）：

```bash
deploy-installer doctor
```

## 3. Linux sudoers（systemd 机器必需）

Linux 上服务注册/启停/日志需要非交互 sudo，在 sudoers 配置受限 NOPASSWD（推荐做法）：

```text
用户名 ALL=(root) NOPASSWD: /usr/bin/systemctl, /usr/bin/journalctl, /usr/bin/install, /usr/bin/mkdir, /usr/bin/rm
```

五枚命令分别覆盖：服务管理（systemctl/journalctl）+ unit 文件写删（install/mkdir/rm）。缺后三枚会在 unit 落盘时挂住。

## 4. 准备上传后端凭证（二选一，必需）

解析产物（Markdown / 图片）需要上传后端，安装前导出对应凭证（也可装完再写入 `~/document-parser/.document-parser.env`）：

```bash
# 方案 A：阿里云 OSS（云端部署）
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
export ALIYUN_OSS_PUBLIC_BUCKET=你的公共bucket     # 与 config.yml 的 storage.oss 对应
export ALIYUN_OSS_PRIVATE_BUCKET=你的私有bucket

# 方案 B：自建系统上传接口（私有部署，nuwax 风格 REST 契约）
export DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://your-system.example.com
export DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=你的APIKey
```

> 方案 A 的 bucket 键可省略——省略时须把真实 bucket 写进 `config.yml` 的
> `storage.oss.public_bucket / private_bucket`；两者都还是模板占位符时 install
> 会 fail-fast 报错（运行期异步上传才会炸 E010 的问题已前移到安装期）。

未配置任何后端时 install 会明确报错并列出配置方式——不会带着坏配置静默“成功”。

## 5. 一键部署

```bash
deploy-installer document-parser install
```

默认安装目录 `~/document-parser`、端口 **8087**；装到其它目录加 `--install-dir`：

```bash
deploy-installer document-parser install --install-dir ~/apps/document-parser
```

venv、配置、任务数据、日志都在安装目录内。**非默认目录时，后续 `upgrade` / `verify` / `service` 子命令都要带同样的 `--install-dir`**（不传时与 install 同默认 `~/document-parser`）。各平台差异：

- **macOS**：自动下载预编译 Python 环境（OSS，约 330MB）；下载源不可达时自动回退 uv 现场构建（耗时数分钟）。**不要**装在 Documents / Desktop / iCloud 目录（launchd 服务权限限制）。
- **Linux**：服务内通过 uv 自动创建 `./venv` 并安装 MinerU/MarkItDown（首次启动后台进行，健康检查最长等 120s，装完前解析任务排队）。
- **Windows**：以当前用户计划任务（任务名 `com.nuwax.document-parser`，S4U 登录、开机自启）注册服务。
- **纯内网**（无法访问 OSS 下载源）：用本地 venv 包离线安装
  `deploy-installer document-parser install --venv-file /path/to/venv-macos-arm64-x.y.z.tar.gz`

命令结束时健康检查通过会打印 `✅ document-parser → http://127.0.0.1:8087`；超时则如实报错并给出平台对应的排障命令（Linux `journalctl -u document-parser -n 30` / Windows 任务计划程序历史 / macOS logs 目录）。

> **SSH 登录的 Mac 特例**：launchd 需要图形会话。纯 SSH 下 plist 已写入、服务等桌面登录后自启，install 以退出码 0 结束并打印手动启动命令——这是有意降级，不是失败。

## 6. 验证

```bash
# 健康检查
curl http://localhost:8087/health

# 解析链路冒烟（同步接口，小文件）
echo "# hello" > /tmp/t.md
curl -X POST "http://localhost:8087/api/v1/documents/parse-sync" -F "file=@/tmp/t.md"

# 接口文档（Swagger UI）
open http://localhost:8087/api/docs

# 接口文档（Scalar 风格，与 Swagger UI 并存；UI JS 由浏览器从公网 CDN 加载）
open http://localhost:8087/api/docs/scalar

# 一键自验（health/ready/文档/parse 冒烟/模型就绪性）
deploy-installer document-parser verify
```

install 默认自动从 OSS 下载 MinerU 模型缓存（~1GB → `~/.cache/modelscope`），
装完即解析 PDF；`--skip-models` 跳过（首跑将从 ModelScope 下载，部分网络慢/卡）。

## 7. 服务管理与升级

```bash
deploy-installer document-parser service status
deploy-installer document-parser service stop       # 停止（幂等：已停视为成功）
deploy-installer document-parser service start      # 启动并等健康检查通过（幂等）
deploy-installer document-parser service restart    # 三平台通用（内建端口释放等待与健康检查）
deploy-installer document-parser service uninstall
deploy-installer document-parser upgrade        # 换新二进制并自动重启在跑的服务
deploy-installer document-parser upgrade --install-dir ~/apps/document-parser   # 非默认目录；verify / service 子命令同理
```

### 7.1 启动 / 停止

stop / start 是普通用户入口，三平台行为一致、无需记平台原生命令：

```bash
deploy-installer document-parser service stop    # 停止；macOS 上是卸载（bootout）语义，start 会重新加载
deploy-installer document-parser service start   # 启动并等待 /health 通过；纯 SSH 无桌面登录的 Mac 见 §5 特例说明
```

两个子命令都**幂等**：已停再 stop、已跑再 start 都打印提示后成功退出。停止会等端口真正释放（Windows 上超时自动强杀残留实例），启动只认健康检查通过（失败如实报错并附日志查看命令）。

各平台底层等价命令（排障时可单独执行，平时用子命令即可）：

| 平台 | 停止 | 启动 |
|------|------|------|
| Linux | `sudo systemctl stop document-parser` | `sudo systemctl start document-parser` |
| macOS | `launchctl bootout gui/$(id -u)/com.nuwax.document-parser` | `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.nuwax.document-parser.plist` |
| Windows | `schtasks /end /tn com.nuwax.document-parser` | `schtasks /run /tn com.nuwax.document-parser` |

- Windows：手动 `/end` 后稍等几秒再 `/run`（旧实例端口释放有窗口期，立即重跑易报端口被占）——子命令已内建该等待与重试。
- macOS：`bootout` 是卸载并停止，之后须 `bootstrap` 重新加载才会启动（`service start` 即做此事）。

### 7.2 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| Linux 启动即挂，日志报 libxcb/libGL 缺失 | 无桌面服务器缺 X11/GL 基础库：Debian/Ubuntu `apt install libxcb1 libxkbcommon-x11-0 libgl1 libglib2.0-0`；RHEL 系 `dnf install libxcb libxkbcommon libXext libXrender mesa-libGL glib2`（doctor 会提前检出） |
| 健康检查超时 | 按报错里的平台命令看服务日志；Linux 首启含 MinerU 环境安装，耐心等或重跑 install |
| Python 环境问题 | 服务目录下 `document-parser troubleshoot` 一键诊断 |
| NVIDIA 机器想用 CUDA 解析 | 安装 NVIDIA 驱动 + CUDA toolkit 后重装；无 GPU 自动走 CPU，不阻塞 |

## 8. 其他安装方式

deploy-installer 之外还有两条路径（详见 [crates/document-parser/INSTALL.md](../../../crates/document-parser/INSTALL.md)）：

- **cargo install**：`cargo install --git https://github.com/nuwax-ai/mcp-proxy document-parser --locked`（不取源码，需 Rust 工具链；Windows 主路径之一）
- **GitHub Releases**：手动下载对应平台产物解压

两者装完后同样执行 `document-parser uv-init` 初始化 Python 引擎，再用内置 `document-parser service install` 注册服务。
