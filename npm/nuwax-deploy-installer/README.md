# nuwax-deploy-installer

Unified deployment CLI for nuwax services. **Binaries are bundled inside this npm package** (no GitHub Release download).

## Quick start (Mac Apple Silicon)

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer doctor
deploy-installer voice-cli install
deploy-installer document-parser install   # 需 OSS_ACCESS_KEY_ID / SECRET
```

> 🇨🇳 国内网络建议先 `npm config set registry https://registry.npmmirror.com`
> （包约 80MB，直连 npmjs 很慢；镜像同步有 10–60 分钟延迟，最新 beta 可能需临时
> `--registry https://registry.npmjs.org` 直连）。

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

- macOS Apple Silicon (`darwin-arm64`) — npm vendor 内置三件套 + LaunchAgent
- Linux x86_64 (`linux-x64`) — npm vendor 内置三件套（systemd 服务管理，CPU 开箱即用）；NVIDIA GPU 可选 OSS CUDA bundle（见 MAINTAINER.md）
- Windows — npm 包不支持；用 `cargo install --git`（见 [INSTALL.md](../../crates/document-parser/INSTALL.md) 方式二）
