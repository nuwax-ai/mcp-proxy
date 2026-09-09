# nuwax-deploy-installer

Unified deployment CLI for nuwax services. **Binaries are bundled inside this npm package** (no GitHub Release download).

## Quick start (Mac Apple Silicon)

```bash
npm install -g nuwax-deploy-installer
deploy-installer doctor
deploy-installer voice-cli install
deploy-installer document-parser install   # 需 OSS_ACCESS_KEY_ID / SECRET
```

> 🇨🇳 国内网络建议先 `npm config set registry https://registry.npmmirror.com`
> （包约 80MB，直连 npmjs 很慢；镜像同步有 10–60 分钟延迟，最新 beta 可能需临时
> `--registry https://registry.npmjs.org` 直连）。

## Commands

```bash
deploy-installer doctor
deploy-installer voice-cli install
deploy-installer document-parser install
deploy-installer voice-cli service status
deploy-installer document-parser service status
```

## Docs

| 文档 | 说明 |
|------|------|
| [deploy-document-parser.md](../../crates/deploy-installer/doc/deploy-document-parser.md) | document-parser 部署指南 |
| [deploy-voice-cli.md](../../crates/deploy-installer/doc/deploy-voice-cli.md) | voice-cli 部署指南（含 Linux GPU 三档） |
| [source-deploy.md](../../crates/deploy-installer/doc/source-deploy.md) | 源码编译路线（自定义 feature / 内网） |
| [mac-mini-quickstart.md](../../crates/deploy-installer/doc/mac-mini-quickstart.md) | Mac 部署与运维 |
| [MAINTAINER.md](../../crates/deploy-installer/doc/MAINTAINER.md) | 维护者：发布、OSS 资产、构建配方 |

## Supported platforms

- macOS Apple Silicon (`darwin-arm64`) — npm vendor 内置三件套 + LaunchAgent，STT Metal 加速开箱即用
- Linux x86_64 (`linux-x64`) — npm vendor 内置三件套（systemd）；voice-cli 三档自动检测（NVIDIA→CUDA / AMD·Intel GPU→Vulkan / 无 GPU→CPU，见 deploy-voice-cli.md）
- Windows x64 (`windows-x64`) — npm vendor 内置 deploy-installer + document-parser + voice-cli（任务计划程序服务管理）
