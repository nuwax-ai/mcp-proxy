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

## 预编译 Whisper ggml（voice-cli，macOS ARM64）

| 文件 | 说明 |
|------|------|
| `whisper-ggml-large-v3-{version}.tar.gz` | **默认**：仅 `models/ggml-large-v3.bin`（~3GB） |
| `whisper-ggml-all-{version}.tar.gz` | 可选：`tiny` … `large-v3` 全档（~5GB+） |

OSS 路径示例：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-large-v3-0.2.1.tar.gz
```

使用（Mac 上 **默认开启**；省略时从 `manifest.json` 读 URL）：

```bash
deploy-installer voice-cli install --install-dir ~/voice-cli
```

全档模型：

```bash
deploy-installer voice-cli install --install-dir ~/voice-cli --models all
```

显式 OSS 前缀：

```bash
deploy-installer voice-cli install \
  --install-dir ~/voice-cli \
  --oss-base https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli
```

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

## 维护者：打包 Whisper ggml 并上传 OSS

任意可联网机器（推荐国内，从 ModelScope 拉模型）：

```bash
# 默认：仅 large-v3（~3GB 下载源）
bash scripts/ci/pack-voice-cli-whisper-ggml.sh 0.2.1

# 全档 tiny … large-v3
bash scripts/ci/pack-voice-cli-whisper-ggml.sh --all 0.2.1

# 使用本地已有 models/
bash scripts/ci/pack-voice-cli-whisper-ggml.sh --local-dir ~/voice-cli/models 0.2.1
```

产物：

| 文件 | 说明 |
|------|------|
| `dist/voice-cli/v{VERSION}/whisper-ggml-large-v3-{VERSION}.tar.gz` | 默认部署包 |
| `dist/voice-cli/v{VERSION}/whisper-ggml-all-{VERSION}.tar.gz` | 全档（`--all`） |
| `*.meta.json` | sha256 / 公开 URL |

上传：

```
oss://nuwa-packages/uploads/voice-cli/whisper-ggml-large-v3-{VERSION}.tar.gz
oss://nuwa-packages/uploads/voice-cli/whisper-ggml-all-{VERSION}.tar.gz
```

验证：

```bash
bash scripts/ci/verify-oss-whisper-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-large-v3-0.2.1.tar.gz
bash scripts/ci/verify-oss-whisper-url.sh --all \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-all-0.2.1.tar.gz
```

## 维护者：npm 发布

**先 beta、后正式**，完整步骤见 [RELEASE.md](./RELEASE.md)。

venv 上传并验证通过后：

```bash
# ① beta
git tag -a deploy-v0.2.1-beta.2 -m "nuwax-deploy-installer 0.2.1-beta.2"
git push origin deploy-v0.2.1-beta.2
# 验证: npm i -g nuwax-deploy-installer@beta

# ② 正式（beta 测通后再打）
git tag -a deploy-v0.2.1 -m "nuwax-deploy-installer 0.2.1"
git push origin deploy-v0.2.1
```

本地 pack（不经 CI）：

```bash
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1-beta.2
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1
```

或手动触发 workflow `Deploy Installer Release`（选 channel=`beta` / `latest`）。

## 后续（voice-cli TTS / Linux CUDA，二期）

TTS/Kokoro 预置包、Linux NVIDIA sherpa CUDA 一键 OSS，将复用同一 `optionalAssets` 模式。

**一期（已落地）**：Mac Mini 通过 `deploy-installer voice-cli install` 拉 OSS **large-v3** 并注册 LaunchAgent。见 [mac-mini-quickstart.md](./mac-mini-quickstart.md)。

当前 Linux CUDA 路径仍见 [crates/voice-cli/deploy/README.md](../../voice-cli/deploy/README.md)。
