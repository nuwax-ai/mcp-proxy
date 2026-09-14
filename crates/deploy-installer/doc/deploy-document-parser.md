# document-parser 部署指南

文档解析服务：把 PDF / Word / Excel / PowerPoint / Markdown 等文件解析成结构化的 Markdown。通过统一安装器 **deploy-installer**（npm 包 `@nuwax-ai/deploy-installer`）一条命令完成安装：下载程序、准备 Python 环境、写好配置 → 注册成开机自启服务 → 等服务就绪，全程自动。

> voice-cli（语音转写/TTS）的部署见姊妹篇 [deploy-voice-cli.md](./deploy-voice-cli.md)。

## 1. 环境要求

| 项目 | Linux | macOS | Windows |
|------|-------|-------|---------|
| 操作系统 | Ubuntu 22.04 或更新（其它发行版需 glibc ≥ 2.35） | 14.0 或更新 | Windows 10 / 11 |
| 硬件 | 内存 8GB+，磁盘 5GB+ 空闲 | 同左（需 Apple 芯片 M 系列） | 同左 |
| 系统库 | 没有桌面环境的服务器需预装几个基础库（见 §7.2，doctor 会提示） | 无需 | 无需 |
| Node.js | 18 或更新版本（用于运行安装器） | 同左 | 同左 |
| GPU（可选） | NVIDIA 显卡可加速 PDF 解析（见 §7.2） | 不适用（用 CPU） | 不适用（用 CPU） |

## 2. 安装 deploy-installer

```bash
npm install -g @nuwax-ai/deploy-installer
```

> 🇨🇳 国内网络建议先配 npm 镜像（安装包约 80MB，直连官方源很慢，npmmirror 实测快 40 倍以上）：
>
> ```bash
> npm config set registry https://registry.npmmirror.com
> ```
>
> 注意：全局安装需要 sudo，而 **sudo 不会读取上面的镜像配置**——要让镜像对 sudo 安装生效，需要把镜像地址直接写在命令后面：
>
> ```bash
> sudo npm install -g @nuwax-ai/deploy-installer --registry https://registry.npmmirror.com
> ```
>
> npmmirror 镜像同步有 10–60 分钟延迟，刚发布的新版可能还搜不到，急用可临时直连官方源。
>
> ⚠️ **镜像上的"最新版"可能滞后好几天**（实测遇到过装出几周前旧版的情况）——装完务必用 `deploy-installer --version` 核对；发现是旧版就带上具体版本号重装，或直连官方源：
>
> ```bash
> sudo npm install -g @nuwax-ai/deploy-installer@0.2.28 --registry https://registry.npmjs.org
> ```

> 📦 **从旧包名升级过来的用户**（之前装过 `nuwax-deploy-installer`）：先卸旧名再装新名（两名字会冲突，直接装新名可能假成功）：
>
> ```bash
> sudo npm uninstall -g nuwax-deploy-installer
> sudo npm install -g @nuwax-ai/deploy-installer --registry https://registry.npmjs.org
> ```
>
> 命令仍是 `deploy-installer`，已部署的服务不受任何影响。

装完跑一下自检（会检查系统、磁盘、依赖库、上传配置是否就绪）：

```bash
deploy-installer doctor
```

## 3. Linux 配置 sudo 免密（Linux 必需，一次搞定）

安装器在 Linux 上注册服务、启停服务、看日志都要用 sudo。为了安装过程不被密码卡住，需要提前配置一次免密权限——**不配置的话 install 会在预检时直接报错**。

把下面命令里的 `你的用户名` 换成实际用户名（终端里输 `whoami` 可以看到），然后**整体复制执行**（会要求输一次密码）：

```bash
echo '你的用户名 ALL=(root) NOPASSWD: /usr/bin/systemctl, /usr/bin/journalctl, /usr/bin/install, /usr/bin/mkdir, /usr/bin/rm' | sudo tee /etc/sudoers.d/nuwax-deploy > /dev/null
sudo chmod 440 /etc/sudoers.d/nuwax-deploy
```

这五行命令只放行部署所需的最小权限：服务的注册/启停/开机自启（systemctl）、看日志（journalctl）、写入和清理服务配置文件（install/mkdir/rm），不会开放其它 root 权限。只在本机部署用，安全风险可控。

## 4. 准备上传凭证（二选一，必需）

解析出来的文件（Markdown / 图片）需要上传到一个存储服务，安装前要准备好对应账号信息（也可以装完再写入 `~/document-parser/.document-parser.env`）：

```bash
# 方案 A：阿里云 OSS（云端部署）
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
export ALIYUN_OSS_PUBLIC_BUCKET=你的公共bucket     # 与 config.yml 的 storage.oss 对应
export ALIYUN_OSS_PRIVATE_BUCKET=你的私有bucket

# 方案 B：自己系统的上传接口（私有部署；接口格式要求见
# crates/document-parser/CUSTOM_UPLOAD_API.md）
export DOCUMENT_PARSER_CUSTOM_UPLOAD_BASE_URL=https://your-system.example.com
export DOCUMENT_PARSER_CUSTOM_UPLOAD_API_KEY=你的APIKey
```

> 方案 A 的两个 bucket 环境变量可以不设——但那样就要把真实 bucket 名写进
> `config.yml` 的 `storage.oss` 部分；两边都没填时 install 会直接报错提醒
> （不会装完之后、上传文件时才发现问题）。

什么都没配置时 install 会明确报错并列出配置方法——不会带着缺的配置"假成功"。

## 5. 一键部署

```bash
deploy-installer document-parser install
```

默认安装目录 `~/document-parser`、端口 **8087**；装到其它目录加 `--install-dir`：

```bash
deploy-installer document-parser install --install-dir ~/apps/document-parser
```

Python 环境、配置、任务数据、日志都在安装目录里。**非默认目录时，后续的 `upgrade` / `verify` / `service` 命令都要带同样的 `--install-dir`**（不带就默认找 `~/document-parser`）。各平台差异：

- **macOS**：自动下载做好的 Python 环境（约 330MB）；下载不了时自动改为联网现场搭建（需要几分钟）。**不要**装在"文稿/桌面/iCloud 同步"目录（系统会禁止服务访问）。
- **Linux**：首次启动时自动创建 Python 环境并安装解析引擎（在后台进行，安装命令最长等 2 分钟；环境装好之前提交的解析任务会排队）。
- **Windows**：注册成计划任务，开机自动运行，无需登录桌面。
- **纯内网**（无法联网下载）：用离线包装
  `deploy-installer document-parser install --venv-file /path/to/venv-macos-arm64-x.y.z.tar.gz`

安装成功会打印 `✅ document-parser → http://127.0.0.1:8087`；如果超时，会如实报错并告诉你怎么查日志（Linux `journalctl -u document-parser -n 30` / Windows 任务计划程序历史 / macOS 看 logs 目录）。

> **只用 SSH 远程连接 Mac 的情况**：Mac 的开机自启要求你本人在这台 Mac 上登录过桌面。远程安装会正常完成（配置都已写好），但要等你登录一次桌面服务才会启动——这是正常行为，不是安装失败。

## 6. 验证

```bash
# 健康检查
curl http://localhost:8087/health

# 快速试一次文档解析（拿个小文件试）
echo "# hello" > /tmp/t.md
curl -X POST "http://localhost:8087/api/v1/documents/parse-sync" -F "file=@/tmp/t.md"

# 接口文档（Swagger 风格）
open http://localhost:8087/api/docs

# 接口文档（另一种风格，与上面并存；页面组件由浏览器从公网加载，纯内网打不开）
open http://localhost:8087/api/docs/scalar

# 一键自验（健康 + 接口文档 + 解析试跑 + 模型就绪检查）
deploy-installer document-parser verify
```

安装时默认自动下载解析模型（约 1GB），装完就能直接解析 PDF；
加 `--skip-models` 可以跳过（之后第一次解析 PDF 时会现场下载，部分网络下较慢）。

## 7. 服务管理与升级

```bash
deploy-installer document-parser service status
deploy-installer document-parser service stop       # 停止（重复执行也安全）
deploy-installer document-parser service start      # 启动（会等服务真正就绪才返回）
deploy-installer document-parser service restart    # 重启（Mac/Linux/Windows 通用）
deploy-installer document-parser service uninstall  # 卸载
deploy-installer document-parser upgrade            # 升级到新版（自动重启在跑的服务）
deploy-installer document-parser upgrade --install-dir ~/apps/document-parser   # 装在非默认目录时要带；verify / service 命令同理
```

### 7.1 启动 / 停止

stop / start 在三个平台用法完全一样，不用记各系统自己的命令：

```bash
deploy-installer document-parser service stop    # 停止（Mac 上会把开机自启也一并注销，start 会自动恢复）
deploy-installer document-parser service start   # 启动并等待服务就绪；纯 SSH 连 Mac 的限制见 §5 说明
```

两个命令**重复执行也安全**：已停止的服务再 stop、已运行的服务再 start，都只是提示一下并正常结束。停止时会等端口真正释放完（Windows 上超时会自动结束残留进程）；启动时只认健康检查真正通过（起不来会明确报错，并告诉你怎么查日志）。

各平台等价的系统原生命令（**仅供排障时使用**，平时用上面的命令即可）：

| 平台 | 停止 | 启动 |
|------|------|------|
| Linux | `sudo systemctl stop document-parser` | `sudo systemctl start document-parser` |
| macOS | `launchctl bootout gui/$(id -u)/com.nuwax.document-parser` | `launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.nuwax.document-parser.plist` |
| Windows | `schtasks /end /tn com.nuwax.document-parser` | `schtasks /run /tn com.nuwax.document-parser` |

- Windows：手动 `/end` 之后要等几秒再 `/run`（旧进程释放端口需要一点时间，立刻重跑容易报端口被占）——上面的子命令已自动处理这个等待。
- macOS：表里的"停止"会把开机自启一起注销，之后必须用表里的"启动"重新注册——这正是推荐用子命令的原因，这些细节它都会自动处理。

### 7.2 常见问题

| 现象 | 原因与处理 |
|------|-----------|
| Linux 启动即挂，日志报 libxcb/libGL 缺失 | 没有桌面环境的服务器缺几个基础库：Debian/Ubuntu `apt install libxcb1 libxkbcommon-x11-0 libgl1 libglib2.0-0`；RHEL 系 `dnf install libxcb libxkbcommon libXext libXrender mesa-libGL glib2`（doctor 命令会提前发现并提示） |
| 健康检查超时 | 按报错里的提示看服务日志；Linux 第一次启动要安装 Python 环境，耐心等或重跑 install |
| Python 环境问题 | 服务目录下执行 `document-parser troubleshoot` 一键诊断 |
| NVIDIA 显卡机器想加速 PDF 解析 | 装好 NVIDIA 驱动和 CUDA 后重新 install；没显卡就自动用 CPU，不影响使用 |

## 8. 其他安装方式

deploy-installer 之外还有两条路径（详见 [crates/document-parser/INSTALL.md](../../../crates/document-parser/INSTALL.md)）：

- **cargo install**：`cargo install --git https://github.com/nuwax-ai/mcp-proxy document-parser --locked`（不取源码，需 Rust 工具链；Windows 主路径之一）
- **GitHub Releases**：手动下载对应平台产物解压

两者装完后同样执行 `document-parser uv-init` 初始化 Python 引擎，再用内置 `document-parser service install` 注册服务。
