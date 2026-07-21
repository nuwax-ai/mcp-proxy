# nuwax-deploy-installer npm 包

## 包名与命令

| 项目 | 值 |
|------|-----|
| npm 包名 | `nuwax-deploy-installer` |
| CLI 命令 | `deploy-installer` |
| Rust crate | `deploy-installer` |

## 安装

```bash
# 正式版
npm install -g nuwax-deploy-installer

# 测试版（验证 CI / 新功能）
npm install -g nuwax-deploy-installer@beta
```

二进制与模板位于包内 `vendor/`，**不从 GitHub Release 下载**（国内可用）。

## 目录结构

```
nuwax-deploy-installer/
├── bin/deploy-installer.js      # Node 垫片 → vendor/<platform>/deploy-installer
└── vendor/
    ├── darwin-arm64/
    │   ├── deploy-installer
    │   └── document-parser
    └── templates/
        ├── manifest.json
        └── document-parser/
            ├── config.example.yml
            ├── .document-parser.env.example
            └── com.nuwax.document-parser.plist   # 参考模板；实际由 CLI 渲染
```

> 说明：LaunchAgent 直接 `exec document-parser`；`.document-parser.env` 由二进制在启动时加载（不覆盖已有环境变量）。旧安装中的 `run-server.sh` 可手动删除。

环境变量（由 Node 垫片注入）：

- `NUWAX_DEPLOY_ROOT` → `vendor/`
- `NUWAX_DEPLOY_VERSION` → package.json version

## 发布（维护者）

**完整流程（先 beta、后正式）见 [RELEASE.md](./RELEASE.md)。**

简表：

| 步骤 | 命令 |
|------|------|
| ① beta | `git tag deploy-v0.2.1-beta.1 && git push origin deploy-v0.2.1-beta.1` |
| ② 验证 | `npm i -g nuwax-deploy-installer@beta` → Mac Mini 测通 |
| ③ 正式 | `git tag deploy-v0.2.1 && git push origin deploy-v0.2.1` |

| Git tag | npm | dist-tag |
|---------|-----|----------|
| `deploy-v0.2.1-beta.N` | `0.2.1-beta.N` | `@beta` |
| `deploy-v0.2.1` | `0.2.1` | `@latest` |

也可在 Actions 里手动跑 `Deploy Installer Release`（选 `channel=beta|latest`）。

本地组装（不发 npm）：

```bash
bash scripts/ci/assemble-nuwax-deploy-installer.sh 0.2.1 aarch64-apple-darwin
bash scripts/ci/smoke-nuwax-deploy-installer.sh /tmp/doc-parser-smoke
cd npm/nuwax-deploy-installer && npm pack
```

预编译 venv（维护者，见 [oss-optional-assets.md](./oss-optional-assets.md)）：

```bash
bash scripts/ci/pack-document-parser-venv-macos-arm64.sh 0.2.1
```

一键发布准备（assemble + smoke + npm pack）：

```bash
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1-beta.1
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1
```
