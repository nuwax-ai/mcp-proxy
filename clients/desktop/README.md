# ai-toolbox（桌面客户端）

一体版桌面客户端：自带并托管 voice-cli / document-parser 两个服务子进程
（随机端口、仅监听 127.0.0.1、零地址配置）。设计文档见仓库根
[docs/DESKTOP_CLIENT_DESIGN.md](../../docs/DESKTOP_CLIENT_DESIGN.md)。

## 构建与运行

```bash
cd clients/desktop
cargo run            # 开发运行（首次构建拉 gpui 依赖树，数分钟）
cargo nextest run    # 单测（含 runtime 配置 YAML 防回归）
cargo clippy --all-targets -- -D warnings
```

无头自检（不点 GUI 走全链路，`NUWAX_DEPLOY_ROOT` 指向 npm 包 vendor）：

```bash
NUWAX_DEPLOY_ROOT=/path/to/npm/package/vendor cargo run -- --smoke          # voice-cli：装→起→健康→停
NUWAX_DEPLOY_ROOT=/path/to/npm/package/vendor cargo run -- --smoke-dp a.pdf # document-parser：装→起→解析→停
```

本目录是**独立 workspace**（根 Cargo.toml 已 exclude）：自己的 Cargo.lock，
不参与主仓的 CI / Docker 构建链。依赖 crates.io 发布版 `gpui-kit`。

## 开发期资产来源

服务的二进制/模板来自 deploy-installer 的 vendor 目录——`deploy_root()`
会从 exe 向上遍历找 `<repo>/npm/deploy-installer/vendor`，开发期零配置。
显式指定则设 `NUWAX_DEPLOY_ROOT=<路径>`（指向 vendor 或 npm 包根）。

whisper 模型按档下载（默认 small）走 hf-mirror.com → huggingface.co 回退；
mineru 模型 / mac 预编译 venv / ffmpeg 走安装器 manifest 的 OSS 资产。

## 目录布局（运行期）

```
<appdata>/ai-toolbox/            # macOS: ~/Library/Application Support/ai-toolbox
├── single-instance.lock
├── services/
│   ├── voice-cli/               # 二进制、config.yml、config.runtime.yml、models/、ffmpeg
│   └── document-parser/
└── ...
```

## 与 gpui-kit 源码联调（可选，勿提交）

Cargo.toml 中加：

```toml
[patch.crates-io]
gpui-kit = { path = "/Users/soddy/Documents/git-workspace/gpui-kit/crates/kit" }
```
