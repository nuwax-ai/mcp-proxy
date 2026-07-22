# deploy-installer 维护者手册

发布 npm、打包 OSS、Linux CUDA 部署。Mac 日常用户请看 [mac-mini-quickstart.md](./mac-mini-quickstart.md)。

---

## 1. npm 包结构

| 项目 | 值 |
|------|-----|
| npm 包名 | `nuwax-deploy-installer` |
| CLI | `deploy-installer` |
| Rust crate | `deploy-installer` |

```
npm/nuwax-deploy-installer/
├── bin/deploy-installer.js      # Node 垫片 → vendor/<platform>/deploy-installer
└── vendor/
    ├── darwin-arm64/            # 一期：Mac 三件套 + voice-cli dylib
    │   ├── deploy-installer
    │   ├── document-parser
    │   ├── voice-cli
    │   ├── libsherpa-onnx-c-api.dylib
    │   └── libonnxruntime*.dylib
    └── templates/
        ├── manifest.json        # OSS 可选资源 URL 模板
        ├── document-parser/
        └── voice-cli/
```

垫片注入：`NUWAX_DEPLOY_ROOT`、`NUWAX_DEPLOY_VERSION`（beta 的 OSS 文件名仍用稳定版 `X.Y.Z`，不含 `-beta`）。

---

## 2. 发布流程（先 beta → Mac 验证 → 正式 latest）

Workflow：[`.github/workflows/deploy-installer-release.yml`](../../../.github/workflows/deploy-installer-release.yml)

| 阶段 | Git tag | npm version | dist-tag | 用户安装 |
|------|---------|-------------|----------|----------|
| Beta | `deploy-v0.2.1-beta.N` | `0.2.1-beta.N` | `@beta` | `npm i -g nuwax-deploy-installer@beta` |
| 正式 | `deploy-v0.2.1` | `0.2.1` | `@latest` | `npm i -g nuwax-deploy-installer` |

规则：

- tag **必须以** `deploy-v` 开头（避免触发 cargo-dist）
- beta 的 version 必须带 `-beta.N`；正式只能是 `X.Y.Z`
- **不要用** `v0.2.1` 这类 tag 发本包

### 发 beta

```bash
git status && git push origin HEAD
git tag -a deploy-v0.2.1-beta.2 -m "nuwax-deploy-installer 0.2.1-beta.2"
git push origin deploy-v0.2.1-beta.2
```

### Mac Mini 验证清单

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer doctor
deploy-installer voice-cli install
curl -fsS http://127.0.0.1:8077/health

export OSS_ACCESS_KEY_ID=... OSS_ACCESS_KEY_SECRET=...
deploy-installer document-parser install
curl -fsS http://127.0.0.1:8087/health
```

详见 [mac-mini-quickstart.md](./mac-mini-quickstart.md)。

### 发正式

```bash
git tag -a deploy-v0.2.1 -m "nuwax-deploy-installer 0.2.1"
git push origin deploy-v0.2.1
```

### 本地组装（不经 CI）

```bash
bash scripts/ci/assemble-nuwax-deploy-installer.sh 0.2.1 aarch64-apple-darwin
bash scripts/ci/smoke-nuwax-deploy-installer.sh /tmp/doc-parser-smoke
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1-beta.2   # assemble + smoke + npm pack
# 发布：NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1 --publish
```

手动触发 Actions：**Deploy Installer Release** → `channel=beta|latest`。

```bash
npm view nuwax-deploy-installer dist-tags
```

---

## 3. OSS 可选资源

大文件不进 npm，由 `vendor/templates/manifest.json` 的 `optionalAssets` 提供 URL。

**`assetVersion`**（与 npm `version` 解耦）：OSS 文件名中的 `{version}` 占位符使用 `assetVersion`（例如 `0.2.1`），beta 包（`0.2.3-beta.N`）可复用同一份 OSS 资源。上传新 OSS 包后手动 bump `assetVersion`。

### Mac（一期）

| 键 | 文件 | 用途 |
|----|------|------|
| `venv.darwin-arm64` | `venv-macos-arm64-{version}.tar.gz` | document-parser Python 环境 |
| `whisperLargeV3.darwin-arm64` | `whisper-ggml-large-v3-{version}.tar.gz` | voice-cli 默认模型 |
| `whisperAll.darwin-arm64` | `whisper-ggml-all-{version}.tar.gz` | 全档模型 |

公开 URL 前缀：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/
```

#### 打包 venv（Mac 上执行）

```bash
bash scripts/ci/pack-document-parser-venv-macos-arm64.sh 0.2.1
# 上传: oss://nuwa-packages/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz
bash scripts/ci/verify-oss-venv-url.sh --extract \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz
```

#### 打包 Whisper ggml

```bash
bash scripts/ci/pack-voice-cli-whisper-ggml.sh 0.2.1          # 默认 large-v3
bash scripts/ci/pack-voice-cli-whisper-ggml.sh --all 0.2.1    # 全档
# 上传: oss://nuwa-packages/uploads/voice-cli/whisper-ggml-large-v3-0.2.1.tar.gz
bash scripts/ci/verify-oss-whisper-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-large-v3-0.2.1.tar.gz
```

### Linux CUDA（二期）

| 键 | 文件 | 说明 |
|----|------|------|
| `voiceCliCuda.linux-x64` | `voice-cli-cuda-linux-x64-{version}.tar.gz` | binary + 4× `.so`，~360MB |

```bash
# 从 dist/.../linux-x64-cuda 打包
bash scripts/ci/pack-voice-cli-cuda-linux-x64.sh 0.2.1
# 上传: oss://nuwa-packages/uploads/voice-cli/voice-cli-cuda-linux-x64-0.2.1.tar.gz
bash scripts/ci/verify-oss-voice-cli-cuda-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/voice-cli-cuda-linux-x64-0.2.1.tar.gz
```

Linux 用户安装（模型需自备或后续扩展 OSS）：

```bash
deploy-installer voice-cli install --install-dir ~/voice-cli
# 可选: --cuda-lib-dir /usr/local/cuda/lib64 --cudnn-lib-dir <path>
```

---

## 4. 路线图摘要

| 能力 | document-parser | voice-cli Mac | voice-cli Linux CUDA |
|------|-----------------|---------------|----------------------|
| `deploy-installer` 子命令 | ✅ | ✅ | ✅ |
| npm vendor 二进制 | ✅ darwin-arm64 | ✅ + dylib | ❌（走 OSS bundle） |
| 大依赖 OSS | venv | Whisper | CUDA bundle |
| 服务管理 | LaunchAgent / systemd | 同左 | systemd + cuda drop-in |

**未做（三期）**：TTS/Kokoro OSS、npm `vendor/linux-x64/`、同机 GPU 共存调优文档、Linux Whisper OSS manifest。

---

## 5. 发布前检查

- [ ] `feat-deploy` / 发布分支已 push
- [ ] `manifest.json` 中 URL 与 OSS 实际上传版本一致
- [ ] venv + whisper-large-v3 已上传并 verify 通过
- [ ] Linux CUDA 包（若发二期）已 verify
- [ ] GitHub `NPM_TOKEN` 已配置
- [ ] Mac Mini beta 全流程测通后再打 `deploy-vX.Y.Z` 正式 tag
