# RMCP-PROXY 

## 项目简介

本项目实现了一个 mcp 代理服务，用户可以通过 SSE（Server-Sent Events）协议，配置我们提供的 URL 地址，远程使用服务器提供的 mcp 功能。

### 主要功能
- 支持通过 SSE 协议与客户端通信，实时推送数据。
- 支持动态添加 mcp 插件：只需在 mcp 社区查找所需插件，复制对应的 JSON 配置，粘贴到本服务的配置中，即可自动加载并启用插件。
- 每个插件配置完成后，服务器会自动启动对应的 mcp 服务，并生成可供访问的 SSE 协议 URL 地址。
- 用户可通过该 URL 地址，直接使用远程服务器的 mcp 能力。

### 使用流程
1. 在 mcp 社区查找并复制所需插件的 JSON 配置。
2. 将 JSON 配置添加到本服务的插件配置中。
3. 服务器自动加载插件并启动服务，生成对应的 SSE URL。
4. 客户端通过该 URL 地址，即可实时获取 mcp 服务推送的数据。

---

## 环境设置

### 安装 Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 安装 VSCode 插件

- crates: Rust 包管理
- Even Better TOML: TOML 文件支持
- Better Comments: 优化注释显示
- Error Lens: 错误提示优化
- GitLens: Git 增强
- Github Copilot: 代码提示
- indent-rainbow: 缩进显示优化
- Prettier - Code formatter: 代码格式化
- REST client: REST API 调试
- rust-analyzer: Rust 语言支持
- Rust Test lens: Rust 测试支持
- Rust Test Explorer: Rust 测试概览
- TODO Highlight: TODO 高亮
- vscode-icons: 图标优化
- YAML: YAML 文件支持

### 安装 cargo generate

cargo generate 是一个用于生成项目模板的工具。它可以使用已有的 github repo 作为模版生成新的项目。

```bash
cargo install cargo-generate
```

在我们的课程中，新的项目会使用 `tyr-rust-bootcamp/template` 模版生成基本的代码：

```bash
cargo generate tyr-rust-bootcamp/template
```

### 安装 pre-commit

pre-commit 是一个代码检查工具，可以在提交代码前进行代码检查。

```bash
pipx install pre-commit
```

安装成功后运行 `pre-commit install` 即可。

### 安装 Cargo deny

Cargo deny 是一个 Cargo 插件，可以用于检查依赖的安全性。

```bash
cargo install --locked cargo-deny
```

### 安装 typos

typos 是一个拼写检查工具。

```bash
cargo install typos-cli
```

### 安装 git cliff

git cliff 是一个生成 changelog 的工具。

```bash
cargo install git-cliff
```

### 安装 cargo nextest

cargo nextest 是一个 Rust 增强测试工具。

```bash
cargo install cargo-nextest --locked
```
