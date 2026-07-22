# nuwax-deploy-installer

Unified deployment CLI for nuwax services. **Binaries are bundled inside this npm package** (no GitHub Release download).

## Quick start (Mac Apple Silicon)

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer doctor
deploy-installer voice-cli install
deploy-installer document-parser install   # 需 OSS_ACCESS_KEY_ID / SECRET
```

完整步骤：[mac-mini-quickstart.md](../../crates/deploy-installer/doc/mac-mini-quickstart.md)

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
| [mac-mini-quickstart.md](../../crates/deploy-installer/doc/mac-mini-quickstart.md) | Mac 部署与运维 |
| [MAINTAINER.md](../../crates/deploy-installer/doc/MAINTAINER.md) | 发布、OSS、Linux CUDA |

## Supported platforms

- macOS Apple Silicon (`darwin-arm64`) — npm vendor + LaunchAgent
- Linux x86_64 + NVIDIA — OSS CUDA bundle + systemd（见 MAINTAINER.md）
