# deploy-installer 文档

统一部署 CLI（`nuwax-deploy-installer` npm 包）的用户文档。

| 文档 | 说明 |
|------|------|
| [mac-mini-quickstart.md](./mac-mini-quickstart.md) | **Mac Mini 小白部署**（推荐入口） |
| [RELEASE.md](./RELEASE.md) | **维护者发布**：先 beta → 验证 → 正式 latest |
| [npm-package.md](./npm-package.md) | npm 包结构与国内安装 |
| [oss-optional-assets.md](./oss-optional-assets.md) | 可选 OSS 资源（预编译 venv 等） |
| [troubleshooting-mac.md](./troubleshooting-mac.md) | Mac 常见问题 |

## 与各服务内置命令的关系

- **小白用户**：`npm i -g nuwax-deploy-installer` → `deploy-installer document-parser install`
- **高级用户**：仅二进制时仍可用 `document-parser service install`（底层同一套 `deploy-installer` 库）

## 平台支持（第一期）

| 平台 | npm vendor 目录 | 服务管理 |
|------|-----------------|----------|
| macOS Apple Silicon | `vendor/darwin-arm64/` | launchd LaunchAgent |
| Linux x86_64 | 规划中 | systemd |
