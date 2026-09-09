# deploy-installer 文档索引

统一部署 CLI（npm 包 `nuwax-deploy-installer`，命令 `deploy-installer`）。

## 我该看哪份？

| 场景 | 文档 |
|------|------|
| **部署 document-parser（文档解析服务）** | [deploy-document-parser.md](./deploy-document-parser.md) |
| **部署 voice-cli（语音转写/TTS 服务）** | [deploy-voice-cli.md](./deploy-voice-cli.md) |
| **Mac Mini 日常部署 / 运维** | [mac-mini-quickstart.md](./mac-mini-quickstart.md) |
| **发布 npm、打包 OSS、Linux CUDA/Vulkan 构建** | [MAINTAINER.md](./MAINTAINER.md) |

## 平台支持（当前）

| 平台 | 状态 | 说明 |
|------|------|------|
| macOS Apple Silicon | ✅ | npm `vendor/darwin-arm64/`，LaunchAgent，Metal STT + OSS venv/Whisper |
| Linux x86_64 | ✅ | systemd；voice-cli 三档自动检测（CUDA / Vulkan / CPU），见 [deploy-voice-cli.md](./deploy-voice-cli.md) |
| Windows x64 | ✅ | npm `vendor/windows-x64/`，任务计划程序（S4U）服务 |

## 命令关系

- **推荐**：`npm i -g nuwax-deploy-installer`（`@latest`）→ `deploy-installer voice-cli|document-parser …`；尝鲜用 `@beta`
- **高级**：单独二进制仍可用 `voice-cli service` / `document-parser service`（同一套渲染逻辑）
