# nuwax-deploy-installer 发布流程

约定：**先发 beta → Mac Mini 验证 → 再发正式（latest）**。

CI workflow：[`Deploy Installer Release`](../../../.github/workflows/deploy-installer-release.yml)  
npm 包：[`nuwax-deploy-installer`](https://www.npmjs.com/package/nuwax-deploy-installer)

## Tag ↔ npm 对照

| 阶段 | Git tag | npm version | npm dist-tag | 用户安装 |
|------|---------|-------------|--------------|----------|
| Beta | `deploy-v0.2.1-beta.1` | `0.2.1-beta.1` | `@beta` | `npm i -g nuwax-deploy-installer@beta` |
| 正式 | `deploy-v0.2.1` | `0.2.1` | `@latest` | `npm i -g nuwax-deploy-installer` |

规则：

- tag **必须以** `deploy-v` 开头（避免触发 cargo-dist 的 `Release` / `Release Beta`）
- **beta**：version 必须带 `-beta.N`
- **正式**：version 只能是 `X.Y.Z`，不能带 prerelease 后缀
- CI 会校验 channel 与 version 形状是否一致，不一致直接失败

## 推荐流程（以 0.2.1 为例）

### 0. 前置

- [ ] `feat-deploy`（或发布分支）已包含要发布的代码并 push
- [ ] 预编译 venv 已上传 OSS，URL 与 `vendor/templates/manifest.json` 一致  
  当前：`https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/venv-macos-arm64-0.2.1.tar.gz`
- [ ] voice-cli Whisper **large-v3** 包已上传 OSS（`whisper-ggml-large-v3-{X.Y.Z}.tar.gz`）  
  当前模板：`https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/whisper-ggml-large-v3-0.2.1.tar.gz`
- [ ] Secrets：`NPM_TOKEN` 已配置

### 1. 发 beta

```bash
# 确认工作区干净、已 push
git status
git push origin HEAD

# 打 beta tag（指向当前 HEAD）
git tag -a deploy-v0.2.1-beta.2 -m "nuwax-deploy-installer 0.2.1-beta.2"
git push origin deploy-v0.2.1-beta.2
```

观察 Actions：`Deploy Installer Release` 应成功；`Release` / `Release Beta` **不应**再被触发。

### 2. 验证 beta（Mac Mini）

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer --version
# 期望: 0.2.1-beta.N

deploy-installer doctor

# 推荐走预编译 venv（跳过漫长 uv-init）
deploy-installer document-parser install \
  --install-dir ~/document-parser \
  --use-prebuilt-venv

deploy-installer document-parser service status --install-dir ~/document-parser
# 确认 LaunchAgent 直接启动二进制（无 run-server.sh）
plutil -p ~/Library/LaunchAgents/com.nuwax.document-parser.plist | head -40
curl -fsS http://127.0.0.1:8087/health
```

验证清单：

- [ ] `doctor` 通过
- [ ] setup / install 成功
- [ ] launchd 服务 running
- [ ] `/health` 正常
- [ ] OSS 密钥与 config（`mineru.device: mps`）正确

### 3. 发正式 0.2.1

beta 验证通过后：

```bash
# 同一提交或已合并的发布提交上打正式 tag（无 -beta 后缀）
git tag -a deploy-v0.2.1 -m "nuwax-deploy-installer 0.2.1"
git push origin deploy-v0.2.1
```

CI 将：

1. 构建 `aarch64-apple-darwin` 二进制并打进 npm 包
2. `npm publish --tag latest`
3. 显式 `npm dist-tag add nuwax-deploy-installer@0.2.1 latest`
4. 跑 smoke test

### 4. 正式安装（小白用户）

```bash
npm install -g nuwax-deploy-installer
# 或国内镜像
npm install -g nuwax-deploy-installer --registry=https://registry.npmmirror.com

deploy-installer document-parser install --install-dir ~/document-parser
```

文档见 [mac-mini-quickstart.md](./mac-mini-quickstart.md)。

## 手动触发（workflow_dispatch）

GitHub → Actions → **Deploy Installer Release** → Run workflow：

| 场景 | version | channel |
|------|---------|---------|
| 再发一个 beta | `0.2.1-beta.3` | `beta` |
| 发正式 | `0.2.1` | `latest` |

channel 与 version 形状不匹配时 CI 会失败（防止误把 beta 发成 latest）。

## 本地不经 CI 的准备（可选）

```bash
# 只 pack，不 publish
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1-beta.2

# 本地 publish（需 NPM_TOKEN；一般仍推荐走 CI）
NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1-beta.2 --publish
NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.1 --publish
```

## 查 dist-tag

```bash
npm view nuwax-deploy-installer dist-tags
npm view nuwax-deploy-installer versions --json
```

期望正式发布后类似：

```text
{ beta: '0.2.1-beta.2', latest: '0.2.1' }
```

## 注意

1. **不要**用 `v0.2.1` / `0.2.1-beta.1` 这类 tag 发 deploy-installer（那是 cargo-dist / mcp-stdio-proxy 的约定）。
2. 包**第一次**发布时，npm 可能把 `@latest` 也指到 beta；正式 `deploy-v0.2.1` 发布后会把 `@latest` 纠正到稳定版。
3. 正式版与 beta 可共用同一份 OSS venv（文件名含 `0.2.1`，不含 `-beta`）；若依赖大变，另打 venv 并改 manifest。
