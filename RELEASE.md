# mcp-proxy 发布指南

本文档说明如何使用 cargo-dist 进行多平台发布。

## 📦 支持的项目

当前使用 cargo-dist 发布以下二进制文件：
- `mcp-stdio-proxy` (mcp-proxy)
- `document-parser`
- `voice-cli`

## 🚀 发布流程

### 1. 更新版本号

更新对应子项目的 `Cargo.toml` 中的版本号：

```bash
# 更新 mcp-proxy
cd mcp-proxy
# 编辑 Cargo.toml 中的 version 字段

# 更新 document-parser
cd document-parser
# 编辑 Cargo.toml 中的 version 字段

# 更新 voice-cli
cd voice-cli
# 编辑 Cargo.toml 中的 version 字段
```

### 2. 测试本地构建

在发布前，可以测试本地构建是否正常：

```bash
# 生成发布计划
~/.cargo/bin/dist plan --tag=v0.2.0

# 构建（测试）
~/.cargo/bin/dist build --tag=v0.2.0
```

### 3. 提交并打标签

```bash
# 提交更改
git add .
git commit -m "release: v0.2.0"

# 创建标签
git tag v0.2.0

# 推送到远程仓库
git push
git push --tags
```

### 4. GitHub Actions 自动发布

推送标签后，GitHub Actions 会自动：
- ✅ 构建所有目标平台的二进制文件
- ✅ 生成安装脚本（shell 和 powershell）
- ✅ 生成校验和（SHA256）
- ✅ 创建 GitHub Release
- ✅ 上传所有构建产物

## 📦 支持的平台

- Linux x86_64 (`x86_64-unknown-linux-gnu`)
- Linux ARM64 (`aarch64-unknown-linux-gnu`)
- macOS Intel (`x86_64-apple-darwin`)
- macOS Apple Silicon (`aarch64-apple-darwin`)
- Windows x86_64 (`x86_64-pc-windows-msvc`)

## 📥 用户安装方式

### 方式 1: 使用安装脚本（推荐）

**Linux/macOS:**
```bash
# 安装 mcp-proxy
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.sh | sh

# 安装 document-parser
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.sh | sh

# 安装 voice-cli
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/voice-cli-installer.sh | sh
```

**Windows (PowerShell):**
```powershell
# 安装 mcp-proxy
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.ps1 | iex

# 安装 document-parser
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.ps1 | iex

# 安装 voice-cli
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/voice-cli-installer.ps1 | iex
```

### 方式 2: 直接下载二进制

从 [GitHub Releases](https://github.com/nuwax-ai/mcp-proxy/releases) 下载对应平台的压缩包。

**Linux/macOS:**
```bash
# 下载并解压
curl -L https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-x86_64-unknown-linux-gnu.tar.xz | tar xJ

# 安装
sudo mv mcp-stdio-proxy /usr/local/bin/
```

**Windows:**
下载 `.zip` 文件并解压，将 `.exe` 文件放到 PATH 目录中。

### 方式 3: 使用 cargo install

```bash
cargo install mcp-stdio-proxy
```

### 方式 4: 使用 cargo-binstall

```bash
# 如果已安装 cargo-binstall
cargo binstall mcp-stdio-proxy

# 或从 crates.io 直接安装
cargo install cargo-binstall
cargo binstall mcp-stdio-proxy
```

## 🔐 校验和验证

每个发布都会包含 `sha256.sum` 文件，用于验证下载的文件完整性：

```bash
# 下载校验和文件
curl -L -O https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/sha256.sum

# 验证下载的文件
sha256sum -c sha256.sum
```

## 📋 发布检查清单

在发布前，请确认：

- [ ] 所有子项目的版本号已更新
- [ ] `Cargo.toml` 中的 `repository` 字段正确
- [ ] CHANGELOG.md 已更新（如有）
- [ ] 本地测试构建成功
- [ ] Git tag 格式正确（如 `v0.2.0`）
- [ ] 推送 tag 到远程仓库

## 🛠️ 配置文件

### `dist-workspace.toml`

配置 cargo-dist 的行为：

```toml
[dist]
# CI 后端
ci = "github"

# 安装脚本类型
installers = ["shell", "powershell"]

# 构建目标平台
targets = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
]
```

### `.github/workflows/release.yml`

自动生成的 GitHub Actions 工作流，处理：
- 构建计划
- 多平台编译
- 生成安装脚本和校验和
- 创建 GitHub Release

## 📞 故障排查

### 构建失败

如果 GitHub Actions 构建失败，检查：
1. 所有子项目的 `Cargo.toml` 都有 `repository` 字段
2. 版本号格式正确（遵循 SemVer）
3. 所有依赖项兼容

### 本地测试

在本地测试发布流程：

```bash
# 查看发布计划
dist plan --tag=v0.2.0

# 构建产物
dist build --tag=v0.2.0

# 查看构建结果
ls -la target/distrib/
```

## 📚 相关资源

- [cargo-dist 官方文档](https://axodotdev.github.io/cargo-dist/)
- [GitHub Releases](https://github.com/nuwax-ai/mcp-proxy/releases)
- [项目仓库](https://github.com/nuwax-ai/mcp-proxy)
