# deploy-installer 文档索引

统一部署 CLI（npm 包 `nuwax-deploy-installer`，命令 `deploy-installer`）。

## 我该看哪份？

| 场景 | 文档 |
|------|------|
| **Mac Mini 日常部署 / 运维** | [mac-mini-quickstart.md](./mac-mini-quickstart.md) |
| **发布 npm、打包 OSS、Linux CUDA** | [MAINTAINER.md](./MAINTAINER.md) |

## 平台支持（当前）

| 平台 | 状态 | 说明 |
|------|------|------|
| macOS Apple Silicon | ✅ 一期 | npm `vendor/darwin-arm64/`，LaunchAgent，OSS venv + Whisper |
| Linux x86_64 + NVIDIA | ✅ 二期 | OSS CUDA bundle + systemd（见 [MAINTAINER.md](./MAINTAINER.md)） |

## 命令关系

- **推荐**：`npm i -g nuwax-deploy-installer`（`@latest`）→ `deploy-installer voice-cli|document-parser …`；尝鲜用 `@beta`
- **高级**：单独二进制仍可用 `voice-cli service` / `document-parser service`（同一套渲染逻辑）
