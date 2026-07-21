# nuwax-deploy-installer npm 包

## 包名与命令

| 项目 | 值 |
|------|-----|
| npm 包名 | `nuwax-deploy-installer` |
| CLI 命令 | `deploy-installer` |
| Rust crate | `deploy-installer` |

## 安装

```bash
npm install -g nuwax-deploy-installer
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
            └── run-server.sh
```

环境变量（由 Node 垫片注入）：

- `NUWAX_DEPLOY_ROOT` → `vendor/`
- `NUWAX_DEPLOY_VERSION` → package.json version

## 发布（维护者）

打 tag 触发 CI：

```bash
git tag deploy-v0.2.1
git push origin deploy-v0.2.1
```

或手动触发 workflow `Deploy Installer Release`。

本地组装（不发 npm）：

```bash
bash scripts/ci/assemble-nuwax-deploy-installer.sh 0.2.1 aarch64-apple-darwin
bash scripts/ci/smoke-nuwax-deploy-installer.sh /tmp/doc-parser-smoke
cd npm/nuwax-deploy-installer && npm pack
```

## Beta 测试

```bash
npm pack ./npm/nuwax-deploy-installer
npm install -g nuwax-deploy-installer-0.2.1.tgz
deploy-installer doctor
```
