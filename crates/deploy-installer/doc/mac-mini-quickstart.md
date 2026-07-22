# Mac Mini 快速部署 document-parser

适用于 **Apple Silicon（M 系列）Mac Mini**，通过 npm 安装统一部署 CLI，**无需访问 GitHub Release**（二进制已打进 npm 包）。

> **版本说明（2026-07）**  
> - 验证 / 新能力（去掉 `run-server.sh`、进程内加载 `.env`）：请装 **`nuwax-deploy-installer@beta`（≥ 0.2.3-beta.1）**，或等正式版 **`0.2.3`**。  
> - 默认 `npm install -g nuwax-deploy-installer` 会装 **`@latest`**（在 `0.2.3` 发布前仍是 `0.2.2`，行为偏旧）。

## 前置条件

```bash
# 命令行工具（若未安装）
xcode-select --install

# Apple Silicon 上确保 Homebrew 在 PATH（SSH / 非登录壳常缺这一步）
eval "$(/opt/homebrew/bin/brew shellenv)"

# Node.js 18+ 与 uv（有预编译 venv 时 uv 仅作兜底）
brew install node uv
```

国内 npm 可选用镜像：

```bash
npm config set registry https://registry.npmmirror.com
```

## 安装目录注意

请把服务装到 **家目录** 下（文档推荐 `~/document-parser`），**不要**装到：

- `~/Documents/...`、`~/Desktop/...`、iCloud 同步目录（macOS 会拦截 LaunchAgent，报 `Operation not permitted`）
- 网络盘 / 无执行权限的路径

## LaunchAgent 前提（重要）

`service install` 注册的是 **用户级 LaunchAgent**，绑定当前用户的 **图形界面会话（`gui/<uid>`）**：

- 执行安装的用户（如 `soddy`）必须已在本机 **桌面登录**（可锁屏，但不要只停在登录窗口）。
- 若控制台登录的是别人（如 `louis`），你只通过 SSH 以 `soddy` 登录，**没有** `gui/soddy`，`service install` 会失败（常见 exit 134 / `Domain does not support specified action`）。
- 纯 SSH 场景可先手工验证服务（见下文「SSH 临时验证」），自启等坐到机器前用该用户登录桌面后再 `service install`。

## 一键部署（推荐）

使用 OSS **预编译 venv**（约 300MB，远快于本地 `uv-init`）：

```bash
eval "$(/opt/homebrew/bin/brew shellenv)"

# 1. 安装 CLI
# 验证新版本 / 当前推荐：
npm install -g nuwax-deploy-installer@beta
# 正式版（0.2.3 发布后）：
# npm install -g nuwax-deploy-installer

deploy-installer --version   # 期望 ≥ 0.2.3-beta.1（或正式 0.2.3）

# 2. 初始化 + 下载 venv（会请求同系列稳定版包名，如 0.2.3-beta.1 → venv-…-0.2.3.tar.gz）
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv

# 3. 填写 OSS 密钥（不要写 export 前缀）
vim ~/document-parser/.document-parser.env
# OSS_ACCESS_KEY_ID=...
# OSS_ACCESS_KEY_SECRET=...

# 4. 在「本用户已桌面登录」的前提下：注册并启动 LaunchAgent
#    （直接 exec document-parser；.env 由二进制启动时加载）
deploy-installer document-parser service install --install-dir ~/document-parser
```

也可在填好密钥后用一条命令（会先 setup，密钥齐全则自动注册服务）：

```bash
deploy-installer document-parser install \
  --install-dir ~/document-parser \
  --use-prebuilt-venv
```

首次启动会做 MinerU / MarkItDown 环境检查，通常 **数秒到一两分钟**；通过后再访问 health。

修改 `.document-parser.env` 后执行 `service restart` 即可生效（无需改 plist）。

旧安装目录若仍有 `run-server.sh`，可手动删除；**0.2.3+** LaunchAgent 已改为直接启动二进制。

### SSH 临时验证（无 gui 会话时）

```bash
# setup + 填好 .env 之后：
~/document-parser/document-parser --config ~/document-parser/config.yml server
# 另开终端：
curl -fsS http://127.0.0.1:8087/health
```

确认正常后，再在桌面登录下执行 `service install`；若已有手工进程，先停掉以免端口占用：

```bash
pkill -f '/Users/[^/]*/document-parser/document-parser' || true
# 或按实际路径：pkill -f "$HOME/document-parser/document-parser"
```

## 验证

```bash
deploy-installer document-parser service status --install-dir ~/document-parser
curl -fsS http://127.0.0.1:8087/health
```

期望类似：

```json
{"code":"0000","message":"操作成功","data":"health"}
```

可选：确认 plist 无 `run-server.sh`：

```bash
plutil -p ~/Library/LaunchAgents/com.nuwax.document-parser.plist | head -40
```

若 `status` 显示 running 但 health 长时间不通：

```bash
tail -f ~/document-parser/logs/launchd.stdout.log
tail -f ~/document-parser/logs/launchd.stderr.log
```

## 分步命令

```bash
eval "$(/opt/homebrew/bin/brew shellenv)"
deploy-installer doctor
deploy-installer document-parser setup --install-dir ~/document-parser --use-prebuilt-venv
deploy-installer document-parser service install --install-dir ~/document-parser
deploy-installer document-parser service restart --install-dir ~/document-parser
deploy-installer document-parser service status --install-dir ~/document-parser
deploy-installer document-parser service uninstall --install-dir ~/document-parser
```

不使用预编译包、本地装 Python 依赖（较慢）：

```bash
deploy-installer document-parser setup --install-dir ~/document-parser
```

## Mac 配置说明

`setup` 会自动将 `config.yml` 中 `mineru.device` 设为 `mps`（Metal GPU）。

- **起服务 / `/health`**：即使 bucket 仍是模板占位，一般也能启动。  
- **真正解析并上传 OSS**：须把 bucket（及 endpoint）改成你的资源：

```yaml
mineru:
  backend: "pipeline"
  device: "mps"
storage:
  oss:
    public_bucket: "你的-bucket"
    private_bucket: "你的-bucket"
```

## 可选：显式指定 OSS 前缀

默认从 npm 包内 `manifest.json` 解析下载地址。也可手动指定：

```bash
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv \
  --oss-base https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser
```

公开 URL 示例（与 CLI 版本对齐；`0.2.3-beta.1` → `0.2.3`）：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.3.tar.gz
```

说明：

- `@beta`（如 `0.2.3-beta.1`）会复用同系列稳定版 venv 文件名（`0.2.3`），**无需**单独打 beta venv。  
- 发新的 `X.Y.Z` / `X.Y.Z-beta.N` 前，须先上传对应的 `venv-macos-arm64-X.Y.Z.tar.gz`，否则 `--use-prebuilt-venv` 会 404。

详见 [oss-optional-assets.md](./oss-optional-assets.md)。

## 升级

```bash
eval "$(/opt/homebrew/bin/brew shellenv)"
# 验证通道：
npm install -g nuwax-deploy-installer@beta
# 或正式版：
# npm update -g nuwax-deploy-installer

deploy-installer document-parser upgrade --install-dir ~/document-parser
deploy-installer document-parser service install --install-dir ~/document-parser
deploy-installer document-parser service restart --install-dir ~/document-parser
```

维护者发布流程见 [RELEASE.md](./RELEASE.md)。更多故障见 [troubleshooting-mac.md](./troubleshooting-mac.md)。
