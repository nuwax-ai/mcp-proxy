# Mac Mini 快速部署 document-parser

适用于 **Apple Silicon（M 系列）Mac Mini**，通过 npm 安装统一部署 CLI，**无需访问 GitHub Release**（二进制已打进 npm 包）。

## 前置条件

```bash
# 命令行工具（若未安装）
xcode-select --install

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

## 一键部署（推荐）

使用 OSS **预编译 venv**（约 300MB，远快于本地 `uv-init`）：

```bash
# 1. 安装 CLI（正式版用默认；验证新版本用 @beta）
npm install -g nuwax-deploy-installer
# npm install -g nuwax-deploy-installer@beta

# 2. 初始化 + 下载 venv
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv

# 3. 填写 OSS 密钥（不要写 export 前缀）
vim ~/document-parser/.document-parser.env
# OSS_ACCESS_KEY_ID=...
# OSS_ACCESS_KEY_SECRET=...

# 4. 注册并启动 LaunchAgent
deploy-installer document-parser service install --install-dir ~/document-parser
```

也可在填好密钥后用一条命令（会先 setup，密钥齐全则自动注册服务）：

```bash
deploy-installer document-parser install \
  --install-dir ~/document-parser \
  --use-prebuilt-venv
```

首次启动会做 MinerU / MarkItDown 环境检查，通常 **十几秒到一两分钟**；通过后再访问 health。

## 验证

```bash
deploy-installer document-parser service status --install-dir ~/document-parser
curl -fsS http://127.0.0.1:8087/health
```

期望类似：

```json
{"code":"0000","message":"操作成功","data":"health"}
```

若 `status` 显示 running 但 health 长时间不通：

```bash
tail -f ~/document-parser/logs/launchd.stdout.log
tail -f ~/document-parser/logs/launchd.stderr.log
```

## 分步命令

```bash
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

## Mac 必改配置

`setup` 会自动将 `config.yml` 中 `mineru.device` 设为 `mps`（Metal GPU）。请确认 OSS bucket 配置正确：

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

公开 URL 示例：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz
```

说明：`@beta` 包（如 `0.2.1-beta.3`）会复用同系列稳定版 venv 文件名（`0.2.1`），无需单独上传 beta venv。

详见 [oss-optional-assets.md](./oss-optional-assets.md)。

## 升级

```bash
npm update -g nuwax-deploy-installer
deploy-installer document-parser upgrade --install-dir ~/document-parser
deploy-installer document-parser service install --install-dir ~/document-parser
deploy-installer document-parser service restart --install-dir ~/document-parser
```

维护者发布流程见 [RELEASE.md](./RELEASE.md)。
