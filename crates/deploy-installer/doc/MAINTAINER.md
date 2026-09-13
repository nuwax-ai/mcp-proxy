# deploy-installer 维护者手册

发布 npm、打包 OSS、Linux CUDA 部署。Mac 日常用户请看 [mac-mini-quickstart.md](./mac-mini-quickstart.md)。

**当前 npm**（发版后）：`@latest` → `0.2.3`，`@beta` 为预发布线（`npm view nuwax-deploy-installer dist-tags` 可查最新）。

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
    ├── darwin-arm64/            # Mac 三件套 + voice-cli dylib（@loader_path/@rpath）
    │   ├── deploy-installer
    │   ├── document-parser
    │   ├── voice-cli
    │   ├── libsherpa-onnx-c-api.dylib
    │   └── libonnxruntime*.dylib
    ├── linux-x64/               # Linux 三件套 + voice-cli .so（RPATH=$ORIGIN，已 strip）
    │   ├── deploy-installer
    │   ├── document-parser
    │   ├── voice-cli
    │   ├── libsherpa-onnx-c-api.so
    │   ├── libsherpa-onnx-cxx-api.so
    │   └── libonnxruntime.so
    ├── windows-x64/             # Windows 切片（.exe；voice-cli 尝试构建，失败降级两件套）
    │   ├── deploy-installer.exe
    │   ├── document-parser.exe
    │   └── voice-cli.exe + *.dll（若构建成功）
    └── templates/
        ├── manifest.json        # OSS 可选资源 URL + assetVersion
        ├── document-parser/
        └── voice-cli/
```

垫片注入环境变量：

| 变量 | 含义 |
|------|------|
| `NUWAX_DEPLOY_ROOT` | `vendor/` 根目录 |
| `NUWAX_DEPLOY_VERSION` | npm 包版本（如 `0.2.3-beta.3`） |

---

## 2. `manifest.json` 与 `assetVersion`

OSS 大文件 URL 模板在 `vendor/templates/manifest.json`（当前 `assetVersion: "0.2.9"`）：

```json
{
  "version": "0.2.9-beta.N",
  "assetVersion": "0.2.9",
  "optionalAssets": {
    "venv": { "darwin-arm64": ".../venv-macos-arm64-{version}.tar.gz" },
    "whisperLargeV3": { "darwin-arm64": ".../whisper-ggml-large-v3-{version}.tar.gz" },
    "whisperAll": { "darwin-arm64": ".../whisper-ggml-all-{version}.tar.gz" },
    "voiceCliCuda": { "linux-x64": ".../voice-cli-cuda-linux-x64-{version}.tar.gz" }
  }
}
```

| 字段 | 作用 |
|------|------|
| `version` | 随 npm 包版本更新（assemble / CI 写入） |
| `assetVersion` | **OSS 文件名**中的 `{version}` 占位符（当前 `0.2.9`） |
| `optionalAssets` | 各平台 URL 模板 |

**规则**：

- beta 发版（`0.2.9-beta.N`）**不必**每次重传 OSS；保持 `assetVersion: "0.2.9"` 即可复用已有包。
- 上传了新版 OSS（如 `venv-macos-arm64-0.2.10.tar.gz`）后，在仓库里 **手动 bump `assetVersion`** 再发 npm；
  **配套动作**：voice-cli 的 whisper/CUDA 资产也要在 OSS 服务端复制（`copy_object`）到新版本命名，
  否则 `{version}` 替换后 URL 指向不存在的对象（0.2.9 发布时已把 whisper-large-v3/CUDA 从 0.2.1 复制过来）。
- venv 重打包：`scripts/ci/pack-document-parser-venv-macos-arm64.sh <版本>`（relocatable + python3.12 +
  mineru 固定版），上传后跑 `scripts/ci/verify-oss-venv-url.sh <url>` 验证。
- `assemble-nuwax-deploy-installer.sh` 只更新 `version`；**不会覆盖**已有 `assetVersion`（缺失时才用 `VERSION` 去掉 prerelease 自动填）。

代码侧：`deploy_asset_version()` 优先读 `assetVersion`，无则回退到 npm 版本去掉 `-beta` 后缀。

---

## 3. 发布流程（先 beta → Mac 验证 → 正式 latest）

Workflow：[`.github/workflows/deploy-installer-release.yml`](../../../.github/workflows/deploy-installer-release.yml)

三段式：`resolve`（版本/渠道守卫）→ `build` 矩阵（`macos-14` → darwin-arm64、
`ubuntu-22.04` → linux-x64、`windows-latest` → windows-x64[voice-cli 失败自动降级两件套]，各自构建 + 原生 smoke + 上传切片 artifact）→
`publish`（合并切片 → 戳版本 → 校验双平台齐全 → `npm publish` → 记录 tarball 体积）。

Linux 构建依赖（ubuntu job 内 apt 安装）：`libclang-dev clang cmake`（whisper-rs
bindgen / sherpa-onnx -sys）+ X11/GL 组合（满足 doctor 的 syslibs 预检）。
Linux 切片 glibc 下限 = ubuntu-22.04 的 2.35；用户侧缺库时 doctor/setup 会 fail-fast
并给出 dnf/apt 安装命令。

| 阶段 | Git tag 示例 | npm version | dist-tag | 用户安装 |
|------|--------------|-------------|----------|----------|
| Beta | `deploy-v0.2.3-beta.3` | `0.2.3-beta.3` | `@beta` | `npm i -g nuwax-deploy-installer@beta` |
| 正式 | `deploy-v0.2.3` | `0.2.3` | `@latest` | `npm i -g nuwax-deploy-installer` |

**Tag 规则**：

- 必须以 `deploy-v` 开头（避免触发 cargo-dist 的 `Release` workflow）
- beta 必须带 `-beta.N`；正式只能是 `X.Y.Z`
- **不要用** `v0.2.3` 这类 tag 发本包

### 发 beta（推荐：打 tag 触发 CI）

```bash
git status && git push origin HEAD
git tag -a deploy-v0.2.3-beta.4 -m "nuwax-deploy-installer 0.2.3-beta.4"
git push origin deploy-v0.2.3-beta.4
```

CI 自动：更新 workspace 版本 → 双平台构建（各 job 内原生 smoke）→ 合并切片 → `npm publish --tag beta`。

### Mac Mini 验证清单

```bash
npm install -g nuwax-deploy-installer@beta
deploy-installer --version    # 期望 0.2.3-beta.N
deploy-installer doctor       # 安装账号须已桌面登录（gui/<uid>）

deploy-installer voice-cli install
curl -fsS http://127.0.0.1:8077/health

export OSS_ACCESS_KEY_ID=... OSS_ACCESS_KEY_SECRET=...
deploy-installer document-parser install
curl -fsS http://127.0.0.1:8087/health
```

注意：

- **≥ `0.2.3-beta.3`** 才包含 `assetVersion` 修复；更早 beta 可能 OSS 404。
- LaunchAgent 需要**执行 install 的用户**在本机 GUI 登录，不能仅靠 SSH（见 [mac-mini-quickstart.md](./mac-mini-quickstart.md)）。

### 发正式

beta 在 Mac Mini 全流程测通后：

```bash
git tag -a deploy-v0.2.3 -m "nuwax-deploy-installer 0.2.3"
git push origin deploy-v0.2.3
```

### 手动触发 Actions（workflow_dispatch）

GitHub → **Deploy Installer Release** → Run workflow：

| version | channel |
|---------|---------|
| `0.2.3-beta.4` | `beta` |
| `0.2.3` | `latest` |

channel 与 version 形状不匹配时 CI 会直接失败。

### 本地组装（不经 CI）

```bash
bash scripts/ci/assemble-nuwax-deploy-installer.sh 0.2.3-beta.4 aarch64-apple-darwin
bash scripts/ci/smoke-nuwax-deploy-installer.sh /tmp/doc-parser-smoke
bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.3-beta.4          # assemble + smoke + npm pack
# 发布：NPM_TOKEN=*** bash scripts/ci/publish-nuwax-deploy-installer.sh 0.2.3-beta.4 --publish
```

```bash
npm view nuwax-deploy-installer dist-tags
```

---

## 4. OSS 可选资源

大文件不进 npm。打包脚本里的版本号应使用 **`assetVersion`**（当前 `0.2.13`），不是 npm beta 号。

### Mac（一期）

| manifest 键 | OSS 文件（`{version}` = `assetVersion`） | 用途 |
|-------------|-------------------------------------------|------|
| `venv.darwin-arm64` | `v{version}/venv-macos-arm64-{version}.tar.gz` | document-parser Python 环境 |
| `whisperLargeV3.darwin-arm64` | `v{version}/whisper-ggml-large-v3-{version}.tar.gz` | voice-cli 默认模型 |
| `whisperAll.darwin-arm64` | `v{version}/whisper-ggml-all-{version}.tar.gz` | 全档模型 |

公开 URL 前缀（**0.2.13 起按版本号目录组织**——控制台按版本分组、清理旧版本
直接删 `v*/` 前缀；`v0.2.13/` 之前的历史资产平铺在服务目录根下，保留不删，
老 npm 包的平铺 URL 仍有效）：

```
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/v{version}/
https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/v{version}/
```

#### 打包 venv（Mac 上执行）

```bash
bash scripts/ci/pack-document-parser-venv-macos-arm64.sh 0.2.13
# 上传: oss://nuwa-packages/uploads/document-parser/v0.2.13/venv-macos-arm64-0.2.13.tar.gz
bash scripts/ci/verify-oss-venv-url.sh --extract \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/document-parser/v0.2.13/venv-macos-arm64-0.2.13.tar.gz
```

上传新 venv 后：改 `manifest.json` 的 `assetVersion` → 发 npm beta 验证。

#### 打包 Whisper ggml

```bash
bash scripts/ci/pack-voice-cli-whisper-ggml.sh 0.2.13          # 默认 large-v3
bash scripts/ci/pack-voice-cli-whisper-ggml.sh --all 0.2.13    # 全档
# 上传: oss://nuwa-packages/uploads/voice-cli/v0.2.13/whisper-ggml-large-v3-0.2.13.tar.gz
bash scripts/ci/verify-oss-whisper-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/v0.2.13/whisper-ggml-large-v3-0.2.13.tar.gz
```

### Linux CUDA（二期）

| manifest 键 | OSS 文件 | 说明 |
|-------------|----------|------|
| `voiceCliCuda.linux-x64` | `v{version}/voice-cli-cuda-linux-x64-{version}.tar.gz` | binary（whisper ggml-cuda 静态链入）+ libsherpa CPU `.so` ×2 + onnxruntime GPU providers `.so` ×2，~370MB |

**构建配方**（编译机 192.168.32.226：GTX 1080 Ti + CUDA 12.6 toolkit + driver 580——有真卡可本机实测 GPU 推理；编译期无需 N 卡可用任意带 nvcc 的 Linux x86_64）：

```bash
# 一次性依赖：CUDA toolkit（/usr/local/cuda，apt nvidia-cuda-toolkit 或官方 runfile）
# 注意 cuDNN 非必需：whisper-cuda 只用 cublas；sensevoice 的 GPU providers 仅运行时需要
#（编译期 ort load-dynamic 不链 cudnn）
export PATH="$HOME/.cargo/bin:/usr/local/cuda/bin:$PATH" CUDA_PATH=/usr/local/cuda
# 架构 61=1080Ti/75=20系/80=A6000/86=30系/89=4090（4090 实测须真机；sm_89 与 61 同链路）
RUSTFLAGS="-C link-arg=-Wl,-rpath,\$ORIGIN" \
  GGML_NATIVE=OFF \
  CMAKE_CUDA_ARCHITECTURES="61;75;80;86;89" \
  cargo build --release -p voice-cli --features cuda,sensevoice-cuda
# 验证（必做）
readelf -d target/release/voice-cli | grep RUNPATH   # 应含 $ORIGIN
# GPU 实测（编译机有卡时）：临时目录放 binary+so+config（独立端口）→ server run →
# transcribe → 日志应出现 ggml_cuda_init: found 1 CUDA devices
# providers_cuda/shared 两枚 .so 来自 ort-cuda feature 的构建产物
```

打包/上传/校验（staging 目录需收齐 voice-cli + 4×`.so`）：

```bash
# staging: dist/voice-cli/v{V}/linux-x64-cuda/{voice-cli,libsherpa-onnx-c-api.so,
#          libonnxruntime.so,libonnxruntime_providers_cuda.so,libonnxruntime_providers_shared.so}
bash scripts/ci/pack-voice-cli-cuda-linux-x64.sh 0.2.26
# 上传: oss://nuwa-packages/uploads/voice-cli/v0.2.26/voice-cli-cuda-linux-x64-0.2.26.tar.gz
bash scripts/ci/verify-oss-voice-cli-cuda-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/v0.2.26/voice-cli-cuda-linux-x64-0.2.26.tar.gz
```

Linux 用户安装（whisper 模型三平台同 URL 自动下载，见 manifest `whisperLargeV3` / `whisperAll` 键）：

```bash
deploy-installer voice-cli install --install-dir ~/voice-cli
# 可选: --cuda-lib-dir /usr/local/cuda/lib64 --cudnn-lib-dir <path>
```

### Linux Vulkan（三期）

| manifest 键 | OSS 文件 | 说明 |
|-------------|----------|------|
| `voiceCliVulkan.linux-x64` | `v{version}/voice-cli-vulkan-linux-x64-{version}.tar.gz` | binary（ggml-vulkan 静态链入）+ 2× CPU `.so` + `.voice-cli-vulkan` marker，~60MB |
| `mineruModels.*` | `models/mineru-pipeline-models-pdf-extract-kit-1.0.tar.gz` | PDF-Extract-Kit-1.0 模型缓存（999MB，**平台无关三键同 URL**；解压到 `~/.cache`；URL 无 {version}——模型版本独立于包版本）。来源：任一机器 `tar czf -C ~/.cache modelscope` 打包上传 |

**三档自动检测**（`assets.rs::resolve_linux_tier`，顺序即优先级）：
显式 `--use-oss-cuda`/`--use-oss-vulkan` → 双 skip（强制 CPU）→ 已装档位幂等保留 →
预检（NVIDIA+libcublas → CUDA；Vulkan 探针 → Vulkan；否则 CPU + WARN 回退）。

- **Vulkan GPU 探针**：隐藏子命令 `__probe-vulkan`（ash 标准绑定，零扩展
  instance → 枚举设备 → deviceType 驱动自报，llvmpipe 软件渲染如实报 CPU 型）；
  父进程自 reexec 子进程 + 10s 超时——坏驱动 segfault/死等只影响探针，
  安装器按"无 Vulkan"降级回 CPU，不中断安装
- **marker 文件**：vulkan 二进制与 CPU vendor 版伴生 `.so` 完全相同，按文件
  不可区分——bundle 内 `.voice-cli-vulkan`（内容 `vulkan {VERSION}`）是档位
  唯一判据；档位切换时安装器自动互斥清理（cuda↔vulkan↔cpu）
- **行为变化**：`--skip-oss-cuda` 从"强制 CPU"变为"跳过 cuda 档仍可自动
  vulkan"；强制 CPU 用双 skip `--skip-oss-cuda --skip-oss-vulkan`
- **upgrade installed-first**：已装 CUDA/Vulkan bundle 的机器升级保留档位
  （修"驱动临时不可用时 CPU 二进制覆盖 CUDA 安装"旧问题）

**构建配方**（编译机 192.168.32.226，编译期无需 GPU）：

```bash
# 一次性依赖（glslc 来自 glslang-tools；libvulkan-dev 提供 find_package(Vulkan)）
sudo apt install -y libvulkan-dev glslang-tools
# 构建（rpath $ORIGIN 必须——否则 systemd 下伴生 .so 找不到，启动 127）
RUSTFLAGS="-C link-arg=-Wl,-rpath,\$ORIGIN" \
  cargo build --release -p voice-cli --features vulkan
# 验证（必做）
readelf -d target/release/voice-cli | grep -E 'RUNPATH|RPATH'   # 应含 $ORIGIN
ldd target/release/voice-cli | grep vulkan                      # 应有 libvulkan.so.1 => 系统
# sherpa CPU .so 复用 CUDA 构建产物（libsherpa-onnx-c-api.so / libonnxruntime.so）
```

打包/上传/校验：

```bash
bash scripts/ci/pack-voice-cli-vulkan-linux-x64.sh 0.2.26
# 上传: oss://nuwa-packages/uploads/voice-cli/v0.2.26/voice-cli-vulkan-linux-x64-0.2.26.tar.gz
bash scripts/ci/verify-oss-voice-cli-vulkan-url.sh \
  https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/uploads/voice-cli/v0.2.26/voice-cli-vulkan-linux-x64-0.2.26.tar.gz
```

---

## 5. 路线图摘要

| 能力 | document-parser | voice-cli Mac | voice-cli Linux CUDA |
|------|-----------------|---------------|----------------------|
| `deploy-installer` 子命令 | ✅ | ✅ | ✅ |
| npm vendor 二进制 | ✅ darwin-arm64 | ✅ + dylib | ❌（走 OSS bundle） |
| 大依赖 OSS | venv | Whisper | CUDA / Vulkan bundle |
| 服务管理 | LaunchAgent / systemd | 同左 | systemd + cuda drop-in |

**未做（三期）**：TTS/Kokoro OSS、npm `vendor/linux-x64/`、同机 GPU 共存调优文档、Linux Whisper OSS manifest。

---

## 6. 发布前检查

- [ ] 发布分支已 push
- [ ] `manifest.json` 中 **`assetVersion`** 与 OSS 上实际文件名一致
- [ ] 若只发 CLI 小改、OSS 未变：**不要**误改 `assetVersion`
- [ ] **GPU bundle 与 npm 发版线对齐**：CUDA/Vulkan bundle 是独立手工构建（不走 CI 的
      版本注入）——发版含 voice-cli 代码变更时须确认 OSS 上的 bundle 也用同级代码
      重建过（bundle 内 `--version` 或装后 `/health` 的 version 应与发版线一致）；
      0.2.25 时代曾断代两周（bundle 停在旧代码，GPU 档用户缺修复且版本显示误导）
- [ ] venv + whisper-large-v3（及 Linux CUDA 若相关）已上传并 `verify-oss-*` 通过
- [ ] assetVersion 升版后，whisper / cuda / venv 等旧资产已在 OSS **服务端复制**
      （`copy_object`）到新版本命名（`{version}` 占位替换后 URL 不断链）
- [ ] GitHub `NPM_TOKEN` 已配置
- [ ] Mac Mini：`doctor` + `voice-cli install` +（可选）`document-parser install` 测通
- [ ] Mac Mini 验证使用 **≥ 当前 beta** 且安装账号已 **桌面登录**
- [ ] 正式 `deploy-vX.Y.Z` 仅在 beta 验证通过后打 tag
