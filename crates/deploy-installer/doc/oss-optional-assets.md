# OSS 可选资源

npm 包体积限制下，以下大文件**不进 npm**，通过 `--oss-base` 按需下载。

## 预编译 Python venv（macOS ARM64）

| 文件 | 说明 |
|------|------|
| `venv-macos-arm64-{version}.tar.gz` | mineru 3.4.4 + markitdown + torch(MPS) |

OSS 路径示例：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/document-parser/v0.2.1/venv-macos-arm64-v0.2.1.tar.gz
```

使用（`--oss-base` 可选；省略时从 npm 包内 `manifest.json` 读取 URL）：

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

`vendor/templates/manifest.json` 记录 URL 模板，CI 发布时可更新 version 字段。

## 后续（voice-cli CUDA）

Linux NVIDIA 服务器上的预编译 sherpa CUDA 包将使用相同模式，由 `deploy-installer voice-cli install` 拉取（第二期）。
