# OSS Client

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# OSS Client

简洁易用的阿里云 OSS (Object Storage Service) 操作库，提供基本的文件操作和签名 URL 功能。

## 功能特性

### OssClientTrait (统一接口)
- **统一接口**: 定义了OSS客户端的基本操作接口
- **多态支持**: 支持私有bucket和公有bucket客户端的统一使用
- **代码复用**: 减少重复代码，提高维护性

### OssClient (私有Bucket客户端)
- **文件操作**: 上传、下载、删除文件
- **签名URL**: 生成带过期时间的上传/下载签名URL
- **文件管理**: 检查文件存在性、生成唯一对象键
- **连接测试**: 测试OSS连接状态

### PublicOssClient (公有Bucket客户端)
- **公开访问**: 生成无需签名的公开下载/访问URL
- **批量操作**: 批量生成公开URL
- **文件操作**: 上传、下载、删除文件（使用公有bucket）
- **签名URL**: 生成上传签名URL（使用公有bucket）
- **文件管理**: 检查文件存在性、生成唯一对象键
- **连接测试**: 测试公有bucket连接状态
- **元信息获取**: 获取文件元信息（通过HTTP HEAD请求）

## 安装

在你的 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
oss-client = { path = "../oss-client" }  # 如果在同一个 workspace 中
# 或者
oss-client = "0.1.0"  # 如果发布到 crates.io
```

## 快速开始

### 环境变量配置

```bash
export OSS_ACCESS_KEY_ID="your_access_key_id"
export OSS_ACCESS_KEY_SECRET="your_access_key_secret"
```

### 基本使用

```rust
use oss_client::{OssClient, OssConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 从环境变量创建客户端（推荐方式）
    let client = oss_client::create_client_from_env()?;

    // 上传文件
    let upload_url = client.upload_file("local/document.pdf", "uploads/document.pdf")?;
    println!("文件上传成功: {}", upload_url);

    // 检查文件是否存在
    let exists = client.file_exists("uploads/document.pdf")?;
    println!("文件存在: {}", exists);

    // 下载文件
    client.download_file("uploads/document.pdf", "downloaded/document.pdf")?;
    println!("文件下载成功");

    Ok(())
}
```

## 许可证

MIT OR Apache-2.0

## 贡献

欢迎提交 Issue 和 Pull Request！
