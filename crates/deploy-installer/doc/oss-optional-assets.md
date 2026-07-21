# OSS 可选资源

npm 包体积限制下，以下大文件**不进 npm**，通过 `--oss-base` 按需下载。

## 预编译 Python venv（macOS ARM64）

| 文件 | 说明 |
|------|------|
| `venv-macos-arm64-{version}.tar.gz` | mineru 3.4.4 + markitdown + torch(MPS) |

OSS 路径示例：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz
```

使用（`--oss-base` 可选；省略时从 npm 包内 `manifest.json` 读取 URL）：

```bash
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv
```

或显式指定完整 URL 前缀目录（`--oss-base` 会拼 `/venv-macos-arm64-{version}.tar.gz`）：

```bash
deploy-installer document-parser setup \
  --install-dir ~/document-parser \
  --use-prebuilt-venv \
  --oss-base https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser
```

`vendor/templates/manifest.json` 记录 URL 模板，CI 发布时可更新 version 字段。

## 维护者：本地打包 venv 并上传 OSS

在 **Apple Silicon Mac** 上执行（耗时约 10–30 分钟，取决于网络）：

```bash
# 1. 打包（输出到 dist/document-parser/v0.2.1/）
bash scripts/ci/pack-document-parser-venv-macos-arm64.sh 0.2.1

# 仅检查环境，不实际构建
bash scripts/ci/pack-document-parser-venv-macos-arm64.sh --dry-run
```

产物：

| 文件 | 说明 |
|------|------|
| `dist/document-parser/v{VERSION}/venv-macos-arm64-{VERSION}.tar.gz` | 预编译 venv |
| `dist/document-parser/v{VERSION}/venv-macos-arm64-{VERSION}.tar.gz.meta.json` | sha256 / 公开 URL |

手动上传到阿里云 OSS（当前约定路径）：

```
oss://nuwa-packages/uploads/document-parser/venv-macos-arm64-{VERSION}.tar.gz
```

公开 URL：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-{VERSION}.tar.gz
```

上传后验证（可选 `--extract` 做 import 冒烟）：

```bash
bash scripts/ci/verify-oss-venv-url.sh --extract \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz
```

把公开 URL 发给维护者或在 Issue 中确认即可；`manifest.json` 已按该 URL 模板配置。

## 维护者：npm 发布

venv 上传并验证通过后：

```bash
# 本地组装 + 冒烟 + npm pack
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1

# 或直接发布（需 NPM_TOKEN）
NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1 --publish
```

或通过 GitHub tag 触发 CI：

```bash
git tag deploy-v0.2.1
git push origin deploy-v0.2.1
```

## 后续（voice-cli CUDA）

Linux NVIDIA 服务器上的预编译 sherpa CUDA 包将使用相同模式，由 `deploy-installer voice-cli install` 拉取（第二期）。
