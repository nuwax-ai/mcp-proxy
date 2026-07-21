# Mac Mini 快速部署 document-parser

适用于 **Apple Silicon（M 系列）Mac Mini**，通过 npm 安装统一部署 CLI，**无需访问 GitHub Release**（二进制已打进 npm 包）。

## 前置条件

```bash
# 命令行工具（若未安装）
xcode-select --install

# Node.js 18+ 与 uv（Python 环境）
brew install node uv
```

国内 npm 可选用镜像，例如：

```bash
npm config set registry https://registry.npmmirror.com
```

## 一键部署

```bash
# 1. 安装 CLI
npm install -g nuwax-deploy-installer

# 2. 初始化 + 注册服务（推荐单命令）
deploy-installer document-parser install --install-dir ~/document-parser
```

若 OSS 密钥尚未填写，按提示编辑：

```bash
vim ~/document-parser/.document-parser.env
# 填写 OSS_ACCESS_KEY_ID / OSS_ACCESS_KEY_SECRET（不要写 export 前缀）

deploy-installer document-parser service install --install-dir ~/document-parser
```

## 验证

```bash
deploy-installer document-parser service status --install-dir ~/document-parser
curl http://127.0.0.1:8087/health
```

## 分步命令

```bash
deploy-installer doctor
deploy-installer document-parser setup --install-dir ~/document-parser
deploy-installer document-parser service install --install-dir ~/document-parser
deploy-installer document-parser service restart --install-dir ~/document-parser
deploy-installer document-parser service uninstall --install-dir ~/document-parser
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

## 可选：OSS 预编译 venv 加速

首次 `uv-init` 较慢时，可下载预编译 Python 环境（URL 来自 npm 包内 `manifest.json`，也可手动指定 `--oss-base`）：

```bash
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv
```

或显式指定 OSS 前缀：

```bash
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv \
  --oss-base https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/document-parser/v0.2.1
```

详见 [oss-optional-assets.md](./oss-optional-assets.md)。

## 升级

```bash
npm update -g nuwax-deploy-installer
deploy-installer document-parser upgrade --install-dir ~/document-parser
deploy-installer document-parser service restart --install-dir ~/document-parser
```
