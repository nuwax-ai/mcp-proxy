# Mac Mini 快速部署（document-parser + voice-cli）

适用于 **Apple Silicon（arm64）Mac Mini**。安装 CLI 后默认目录为 `~/voice-cli` 与 `~/document-parser`，一般**不必写 `--install-dir`**。

> 请使用 **`npm install -g nuwax-deploy-installer@beta`** 获取最新 Mac 部署能力；稳定版用 `@latest`。

## 最快路径（复制粘贴）

```bash
# ① 一次性准备
xcode-select --install 2>/dev/null || true
eval "$(/opt/homebrew/bin/brew shellenv)"
brew install node curl
# 国内可选：npm config set registry https://registry.npmmirror.com

npm install -g nuwax-deploy-installer@beta
deploy-installer doctor

# ② voice-cli（约 3GB 模型，已存在则跳过；无需 OSS 密钥）
deploy-installer voice-cli install
curl -fsS http://127.0.0.1:8077/health

# ③ document-parser（约 300MB venv + 需要业务 OSS 密钥）
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
deploy-installer document-parser install
# 首次启动较慢，install 会等待 /health 最多约 120 秒
curl -fsS http://127.0.0.1:8087/health
```

| 服务 | 目录 | 端口 | API 文档 |
|------|------|------|----------|
| voice-cli | `~/voice-cli` | 8077 | http://127.0.0.1:8077/api/docs |
| document-parser | `~/document-parser` | 8087 | http://127.0.0.1:8087/api/docs |

### 安装前必读

1. **目录**：装在家目录 `~/voice-cli`、`~/document-parser`。不要用 `Documents` / `Desktop` / iCloud（LaunchAgent 会报 `Operation not permitted`）。
2. **桌面登录**：`service install` 需要本用户已图形界面登录（`doctor` 会检查 `gui/<uid>`）。纯 SSH 见 [附录：SSH 临时验证](#附录ssh-无-gui-时临时验证)。
3. **顺序**：建议先装 **voice-cli**（无密钥），再装 **document-parser**（需 OSS AK/SK）。
4. **默认行为**（Mac 上无需额外参数）：
   - voice-cli：npm 包内二进制 + dylib，**自动**从 OSS 下载 Whisper **large-v3**
   - document-parser：**自动**从 OSS 下载预编译 venv；`config.yml` 中 `mineru.device` 会从 `cpu` 改为 `mps`

---

## 分步说明

### 1. 安装 CLI

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer doctor
deploy-installer --version
```

`doctor` 检查：平台、`vendor/darwin-arm64/` 二进制、voice-cli 伴随 dylib、磁盘空间（约 5GB 余量）、LaunchAgent 所需 GUI 会话。

### 2. voice-cli

```bash
deploy-installer voice-cli install
```

流程：`setup`（复制 binary + dylib + config）→ OSS 下载 `ggml-large-v3.bin`（模型已存在则跳过）→ 注册 LaunchAgent → 等待 `/health`（最多约 **45 秒**）。

### 3. document-parser

**先配置 OSS 密钥**（用于上传解析结果；与下载 venv/模型的公开 OSS 无关）：

```bash
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
deploy-installer document-parser install
```

若未配置密钥，`install` 会完成 setup（venv + 二进制）后 **exit 1** 并提示；配置好后**再执行一次** `install` 即可注册服务。

**方式 B — 配置文件**（适合长期固定密钥）：

```bash
vim ~/document-parser/.document-parser.env
# OSS_ACCESS_KEY_ID=...
# OSS_ACCESS_KEY_SECRET=...
# 注意：不要写 export 前缀

deploy-installer document-parser install
```

---

## 常用运维

```bash
# 状态
deploy-installer voice-cli service status
deploy-installer document-parser service status

# 重启
deploy-installer voice-cli service restart
deploy-installer document-parser service restart

# 升级（先升级 npm 包，再 upgrade 安装目录里的二进制）
npm install -g nuwax-deploy-installer@beta
deploy-installer voice-cli upgrade
deploy-installer document-parser upgrade
deploy-installer voice-cli service restart
deploy-installer document-parser service restart

# 日志
tail -f ~/voice-cli/logs/launchd.stdout.log
tail -f ~/voice-cli/logs/launchd.stderr.log
tail -f ~/document-parser/logs/launchd.stdout.log
tail -f ~/document-parser/logs/launchd.stderr.log
```

---

## 可选参数

```bash
# voice-cli：全档模型 tiny…large-v3（~5GB+）
deploy-installer voice-cli install --models all

# 已有模型 / 离线
deploy-installer voice-cli install --skip-models

# document-parser：不用 OSS venv，本地 uv-init（慢，需 brew install uv python@3.12）
deploy-installer document-parser install --no-prebuilt-venv
```

---

## 故障排查

### `doctor`: missing binary for darwin-arm64

第一期仅支持 **Apple Silicon**。确认 `uname -m` 为 `arm64`，并重装 npm 包：

```bash
npm install -g nuwax-deploy-installer@beta
```

### LaunchAgent 未启动 / `service install` 失败

```bash
deploy-installer voice-cli service status
deploy-installer document-parser service status
launchctl print gui/$(id -u)/com.nuwax.voice-cli 2>&1 | head -20
```

常见原因：

| 现象 | 处理 |
|------|------|
| 仅 SSH、未桌面登录 | 用安装账号登录桌面后再 `install`；或见下方 SSH 临时验证 |
| 目录在 Documents / Desktop | 改到 `~/voice-cli`、`~/document-parser` 后重装 |
| document-parser 密钥未填 | 编辑 `~/.document-parser.env` 或 export 后重新 `install` |
| 端口被占用 | 改 `config.yml` 的 `server.port`，或 `pkill` 手工启动的进程 |
| venv OSS 404 | 升级 `@beta`；维护者确认 OSS 包已上传（见 [MAINTAINER.md](./MAINTAINER.md)） |

### document-parser：health 不通但 status 显示 running

首次启动会做 MinerU/MarkItDown 环境检查，HTTP 可能 **1–2 分钟**后才监听。看 stdout 是否出现 `Service started successfully`：

```bash
tail -f ~/document-parser/logs/launchd.stdout.log
curl -fsS http://127.0.0.1:8087/health
```

确认 LaunchAgent 直接启动二进制（不应有 `run-server.sh`）：

```bash
plutil -p ~/Library/LaunchAgents/com.nuwax.document-parser.plist | head -30
```

### document-parser：解析慢 / CPU 占用高

安装时 CLI 会把 `mineru.device: "cpu"` 改为 `"mps"`。可手动确认：

```bash
grep device ~/document-parser/config.yml
# 期望: device: "mps"
```

### voice-cli：服务起不来

```bash
tail -f ~/voice-cli/logs/launchd.stderr.log
~/voice-cli/voice-cli --version   # 应在安装目录执行
```

确认 dylib 与 binary 同目录：`libsherpa-onnx-c-api.dylib`、`libonnxruntime.1.24.4.dylib`。

### 卸载

```bash
deploy-installer voice-cli service uninstall
deploy-installer document-parser service uninstall
rm -rf ~/voice-cli ~/document-parser   # 可选：删除数据与模型
```

---

## 附录

### OSS 大文件（Mac 用户只需知道来源）

| 服务 | OSS 包 | 大小 | 下载时要密钥？ |
|------|--------|------|----------------|
| voice-cli | `whisper-ggml-large-v3-{version}.tar.gz` | ~3GB | 否（公开 URL） |
| document-parser | `venv-macos-arm64-{version}.tar.gz` | ~300MB | 否 |

业务上传解析结果仍需在 `.document-parser.env` 配置 **OSS_ACCESS_KEY_ID / SECRET**。

维护者打包上传见 [MAINTAINER.md](./MAINTAINER.md)。

### SSH 无 GUI 时临时验证

```bash
~/voice-cli/voice-cli server run --config ~/voice-cli/config.yml
~/document-parser/document-parser --config ~/document-parser/config.yml server
```

回到机器桌面登录后，再执行各服务的 `install` 注册自启。

### Linux / CUDA

见 [MAINTAINER.md](./MAINTAINER.md) 第二节，或 `crates/voice-cli/deploy/README.md`。
