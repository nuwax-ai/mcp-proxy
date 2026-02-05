# cargo-dist 配置完成总结

## ✅ 已完成的配置

### 1. 初始化 cargo-dist

使用官方命令初始化：
```bash
dist init --yes
```

这会自动创建：
- `[profile.dist]` 到 `Cargo.toml`（工作区级别）
- `dist-workspace.toml` 配置文件
- `.github/workflows/release.yml` GitHub Actions 工作流

### 2. 配置文件

#### `dist-workspace.toml`
```toml
[workspace]
members = ["cargo:."]

[dist]
cargo-dist-version = "0.30.3"
ci = "github"
installers = ["shell", "powershell"]
targets = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
]
```

#### 各子项目的 `Cargo.toml` 添加
```toml
[package]
repository = "https://github.com/nuwax-ai/mcp-proxy"
```

已更新：
- ✅ `mcp-proxy/Cargo.toml` (已有 repository)
- ✅ `document-parser/Cargo.toml`
- ✅ `voice-cli/Cargo.toml`

### 3. GitHub Actions 工作流

文件：`.github/workflows/release.yml`

自动执行：
1. **Plan** - 检测到 git tag 时，计算需要构建的内容
2. **Build** - 为每个目标平台构建二进制文件
3. **Host** - 创建 GitHub Release 并上传所有产物

### 4. 产物格式

每次发布会生成：

#### 每个 二进制文件
- `{name}-{target}.tar.xz` (Linux/macOS)
- `{name}-{target}.zip` (Windows)
- 对应的 SHA256 校验和

#### 全局产物
- `{name}-installer.sh` - Shell 安装脚本 (Linux/macOS)
- `{name}-installer.ps1` - PowerShell 安装脚本 (Windows)
- `sha256.sum` - 所有文件的校验和
- `source.tar.gz` - 源码压缩包

## 🚀 发布流程

### 1. 更新版本号
```bash
# 更新各子项目的 version
vim mcp-proxy/Cargo.toml
vim document-parser/Cargo.toml
vim voice-cli/Cargo.toml
```

### 2. 提交并打标签
```bash
git add .
git commit -m "release: v0.2.0"
git tag v0.2.0
git push
git push --tags
```

### 3. 自动触发 GitHub Actions
推送 tag 后，GitHub Actions 自动：
- ✅ 构建所有平台
- ✅ 生成安装脚本
- ✅ 创建 Release
- ✅ 上传产物

## 📥 用户安装方式

### 最简单（推荐）
```bash
# Linux/macOS
curl --proto '=https' --tlsv1.2 -sSf \
  https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.sh | sh

# Windows PowerShell
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.ps1 | iex
```

### 从 Releases 页面下载
访问：https://github.com/nuwax-ai/mcp-proxy/releases

### cargo install
```bash
cargo install mcp-stdio-proxy
```

### cargo-binstall
```bash
cargo binstall mcp-stdio-proxy
```

## 🔧 本地测试发布

```bash
# 查看发布计划
dist plan --tag=v0.2.0

# 构建全局产物（安装脚本）
dist build --tag=v0.2.0 --artifacts=global

# 查看生成的文件
ls -la target/distrib/
```

## 📝 文档

- `RELEASE.md` - 完整的发布指南
- `README.md` - 已更新安装方式章节
- `dist-workspace.toml` - cargo-dist 配置
- `.github/workflows/release.yml` - 自动化工作流

## 🎯 支持的平台

| 平台 | 目标三元组 |
|------|-----------|
| Linux x86_64 | x86_64-unknown-linux-gnu |
| Linux ARM64 | aarch64-unknown-linux-gnu |
| macOS Intel | x86_64-apple-darwin |
| macOS Apple Silicon | aarch64-apple-darwin |
| Windows x86_64 | x86_64-pc-windows-msvc |

## 📦 发布的二进制

1. **mcp-stdio-proxy** - 主 MCP 代理服务
2. **document-parser** - 文档解析服务
3. **voice-cli** - 语音转文字服务

## 🔐 校验和验证

每个 release 都包含 `sha256.sum` 文件：

```bash
# 下载校验和
curl -L -O https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/sha256.sum

# 验证
sha256sum -c sha256.sum
```

## 📚 参考资源

- [cargo-dist 官方文档](https://axodotdev.github.io/cargo-dist/)
- [cargo-dist 配置参考](https://axodotdev.github.io/cargo-dist/book/reference/config.html)
- [项目 GitHub Releases](https://github.com/nuwax-ai/mcp-proxy/releases)
