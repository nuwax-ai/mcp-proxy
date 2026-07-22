# Mac Mini 快速部署（document-parser + voice-cli）

适用于 **Apple Silicon Mac Mini**。安装 CLI 后，**默认目录**为 `~/voice-cli` 与 `~/document-parser`，命令里一般**不必写 `--install-dir`**。

> 新能力请用 **`npm install -g nuwax-deploy-installer@beta`**（≥ `0.2.3-beta.1`）。

## 最快路径（复制粘贴）

```bash
# ① 一次性准备
xcode-select --install
eval "$(/opt/homebrew/bin/brew shellenv)"
brew install node
npm install -g nuwax-deploy-installer@beta
deploy-installer doctor

# ② voice-cli（约 3GB 模型下载，已存在则跳过）
deploy-installer voice-cli install
# 安装结束会自动探测 /health，也可手动：
curl -fsS http://127.0.0.1:8077/health

# ③ document-parser（约 300MB venv + 需业务 OSS 密钥）
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
deploy-installer document-parser install
curl -fsS http://127.0.0.1:8087/health
```

| 服务 | 目录 | 端口 | API 文档 |
|------|------|------|----------|
| voice-cli | `~/voice-cli` | 8077 | http://127.0.0.1:8077/api/docs |
| document-parser | `~/document-parser` | 8087 | http://127.0.0.1:8087/api/docs |

**注意**
- 装在家目录（`~/voice-cli`、`~/document-parser`），不要用 `Documents` / `Desktop`（LaunchAgent 会报 `Operation not permitted`）。
- `service install` 需要**本用户已桌面登录**（`doctor` 会检查 `gui/<uid>`）。纯 SSH 见 [附录](#附录)。

---

## 分步说明

### 1. 安装 CLI

```bash
# 国内可选
npm config set registry https://registry.npmmirror.com

npm install -g nuwax-deploy-installer@beta
deploy-installer doctor
```

`doctor` 会显示默认安装路径、Node/curl 等依赖，以及 LaunchAgent 所需的 GUI 会话。

### 2. voice-cli

默认从 OSS 下载 **Whisper large-v3**（约 **3GB**），注册 LaunchAgent，并等待 `/health` 就绪。

```bash
deploy-installer voice-cli install
```

模型已存在时会**自动跳过下载**（重装 / 升级更快）。

### 3. document-parser

Mac 默认拉 OSS **预编译 venv**（约 300MB）。还需配置**业务 OSS 密钥**（用于上传解析结果）。

```bash
deploy-installer document-parser install
```

若密钥未配置，CLI 会提示。任选一种方式后**再执行一次** `install`：

**方式 A — 环境变量（推荐，适合脚本）**

```bash
export OSS_ACCESS_KEY_ID=你的Key
export OSS_ACCESS_KEY_SECRET=你的Secret
deploy-installer document-parser install
```

**方式 B — 编辑配置文件**

```bash
vim ~/document-parser/.document-parser.env
# OSS_ACCESS_KEY_ID=...
# OSS_ACCESS_KEY_SECRET=...
deploy-installer document-parser install
```

---

## 常用运维

```bash
# 状态（默认目录，无需 --install-dir）
deploy-installer voice-cli service status
deploy-installer document-parser service status

# 重启
deploy-installer voice-cli service restart
deploy-installer document-parser service restart

# 升级 npm 包里的二进制后
npm install -g nuwax-deploy-installer@beta
deploy-installer voice-cli upgrade
deploy-installer document-parser upgrade
deploy-installer voice-cli service restart
deploy-installer document-parser service restart

# 日志
tail -f ~/voice-cli/logs/launchd.stdout.log
tail -f ~/document-parser/logs/launchd.stdout.log
```

---

## 附录

### 对照：OSS 大文件从哪来

| 服务 | OSS 包 | 大小 | 是否需密钥 |
|------|--------|------|------------|
| voice-cli | `whisper-ggml-large-v3-{version}.tar.gz` | ~3GB | 否（公开 URL） |
| document-parser | `venv-macos-arm64-{version}.tar.gz` | ~300MB | 否；但业务上传要 AK/SK |

维护者打包上传见 [oss-optional-assets.md](./oss-optional-assets.md)。

### 可选参数

```bash
# voice-cli：全档模型 tiny…large-v3（~5GB+）
deploy-installer voice-cli install --models all

# 已有模型 / 离线
deploy-installer voice-cli install --skip-models

# document-parser：不用预编译 venv，本地 uv-init（慢）
deploy-installer document-parser install --no-prebuilt-venv
```

### SSH 无 gui 时临时验证

```bash
~/voice-cli/voice-cli server run --config ~/voice-cli/config.yml
~/document-parser/document-parser --config ~/document-parser/config.yml server
```

回到机器桌面登录后，再执行各服务的 `install` 注册自启。

### 更多

- 故障排查：[troubleshooting-mac.md](./troubleshooting-mac.md)
- Linux / CUDA：[voice-cli/deploy/README.md](../../voice-cli/deploy/README.md)
- 发布维护：[RELEASE.md](./RELEASE.md)
