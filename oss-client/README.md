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

## 快速开始

### 安装

在你的 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
oss-client = { path = "../oss-client" }  # 如果在同一个 workspace 中
# 或者
oss-client = "0.1.0"  # 如果发布到 crates.io
```

### 环境变量配置

只需要设置两个环境变量：

```bash
export OSS_ACCESS_KEY_ID="your_access_key_id"
export OSS_ACCESS_KEY_SECRET="your_access_key_secret"
```

其他配置使用默认值：
- **endpoint**: `oss-rg-china-mainland.aliyuncs.com`
- **public_bucket**: `nuwa-packages` (公有bucket)
- **private_bucket**: `edu-nuwa-packages` (私有bucket)
- **region**: `oss-rg-china-mainland`
- **upload_directory**: `edu`

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
    
    // 删除文件
    client.delete_file("uploads/document.pdf")?;
    println!("文件删除成功");
    
    Ok(())
}
```

### 公有Bucket客户端

对于需要公开访问的文件，可以使用 `PublicOssClient`：

```rust
use oss_client::PublicOssClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 从环境变量创建公有bucket客户端
    let client = PublicOssClient::from_env()?;
    
    // 生成公开下载URL（无需签名，永久有效）
    let public_url = client.generate_public_download_url("documents/manual.pdf")?;
    println!("公开下载URL: {}", public_url);
    
    // 生成公开访问URL
    let access_url = client.generate_public_access_url("images/logo.png")?;
    println!("公开访问URL: {}", access_url);
    
    // 批量生成公开URL
    let keys = vec!["doc1.pdf", "doc2.pdf", "image.jpg"];
    let urls = client.generate_public_urls_batch(&keys)?;
    
    // 检查文件是否存在
    let exists = client.file_exists("documents/manual.pdf").await?;
    println!("文件存在: {}", exists);
    
    // 获取文件元信息
    if let Some(metadata) = client.get_file_metadata("documents/manual.pdf").await? {
        println!("文件大小: {} bytes", metadata.get("content-length").unwrap_or(&"未知".to_string()));
        println!("文件类型: {}", metadata.get("content-type").unwrap_or(&"未知".to_string()));
    }
    
    Ok(())
}
```

### 客户端选择指南

| 使用场景 | 推荐客户端 | 特点 |
|----------|------------|------|
| 私有文件、需要安全控制 | `OssClient` | 签名验证、过期时间、安全性高 |
| 公开文件、公网访问 | `PublicOssClient` | 无需签名、永久有效、便利性高 |
| 用户上传、临时分享 | `OssClient` | 可控过期、安全访问 |
| 产品文档、公开资料 | `PublicOssClient` | 永久有效、公开访问 |

### 签名 URL 使用

```rust
use oss_client::OssClient;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = oss_client::create_client_from_env()?;
    
    // 生成上传签名 URL（4小时有效）
    let upload_url = client.generate_upload_url(
        "uploads/document.pdf",
        Duration::from_secs(4 * 3600),
        Some("application/pdf")
    )?;
    println!("上传 URL: {}", upload_url);
    
    // 生成下载签名 URL（1小时有效）
    let download_url = client.generate_download_url(
        "uploads/document.pdf",
        Some(Duration::from_secs(3600))
    )?;
    println!("下载 URL: {}", download_url);
    
    // 生成永久下载 URL
    let permanent_url = client.generate_download_url("uploads/document.pdf", None)?;
    println!("永久 URL: {}", permanent_url);
    
    Ok(())
}
```

## API 文档

### 客户端创建

#### `create_client_from_env() -> Result<OssClient>`

从环境变量创建客户端，这是推荐的方式。

**环境变量要求：**
- `OSS_ACCESS_KEY_ID`: 阿里云访问密钥 ID
- `OSS_ACCESS_KEY_SECRET`: 阿里云访问密钥 Secret

#### `create_client(config: OssConfig) -> Result<OssClient>`

使用自定义配置创建客户端。

#### `OssClient::new(config: OssConfig) -> Result<OssClient>`

直接创建客户端实例。

### 配置管理

#### `OssConfig::from_env() -> Result<OssConfig>`

从环境变量创建配置。

#### `OssConfig::new(access_key_id: String, access_key_secret: String) -> OssConfig`

创建使用默认值的配置。

### 文件操作

#### `upload_file(&self, local_path: &str, object_key: &str) -> Result<String>`

上传本地文件到 OSS。

**参数：**
- `local_path`: 本地文件路径
- `object_key`: OSS 对象键（远程文件路径）

**返回：** 上传成功后的文件 URL

#### `upload_content(&self, content: &[u8], object_key: &str, content_type: Option<&str>) -> Result<String>`

上传字节内容到 OSS。

**参数：**
- `content`: 文件内容字节数组
- `object_key`: OSS 对象键
- `content_type`: MIME 类型（可选，会自动检测）

**返回：** 上传成功后的文件 URL

#### `download_file(&self, object_key: &str, local_path: &str) -> Result<()>`

从 OSS 下载文件到本地。

**参数：**
- `object_key`: OSS 对象键
- `local_path`: 本地保存路径

#### `delete_file(&self, object_key: &str) -> Result<()>`

删除 OSS 文件。

**参数：**
- `object_key`: 要删除的 OSS 对象键

#### `file_exists(&self, object_key: &str) -> Result<bool>`

检查文件是否存在。

**参数：**
- `object_key`: OSS 对象键

**返回：** 文件是否存在

### 签名 URL

#### `generate_upload_url(&self, object_key: &str, expires_in: Duration, content_type: Option<&str>) -> Result<String>`

生成上传签名 URL。

**参数：**
- `object_key`: OSS 对象键
- `expires_in`: 过期时间
- `content_type`: 内容类型（可选）

**返回：** 签名的上传 URL

#### `generate_download_url(&self, object_key: &str, expires_in: Option<Duration>) -> Result<String>`

生成下载签名 URL。

**参数：**
- `object_key`: OSS 对象键
- `expires_in`: 过期时间（None 表示永久有效）

**返回：** 签名的下载 URL

### 工具函数

#### `utils::detect_mime_type(file_path: &str) -> String`

根据文件扩展名检测 MIME 类型。

#### `utils::sanitize_filename(filename: &str) -> String`

清理文件名，移除特殊字符。

#### `utils::generate_unique_filename(prefix: &str, extension: Option<&str>) -> String`

生成唯一文件名。

#### `utils::format_file_size(size: u64) -> String`

格式化文件大小为可读字符串。

## 错误处理

库使用 `thiserror` 定义了完整的错误类型：

```rust
use oss_client::{OssError, Result};

match client.upload_file("test.txt", "uploads/test.txt") {
    Ok(url) => println!("上传成功: {}", url),
    Err(OssError::Config(msg)) => eprintln!("配置错误: {}", msg),
    Err(OssError::Network(msg)) => eprintln!("网络错误: {}", msg),
    Err(OssError::FileNotFound(msg)) => eprintln!("文件不存在: {}", msg),
    Err(OssError::Permission(msg)) => eprintln!("权限不足: {}", msg),
    Err(e) => eprintln!("其他错误: {}", e),
}
```

### 错误类型

- `Config(String)` - 配置错误
- `Network(String)` - 网络错误
- `FileNotFound(String)` - 文件不存在
- `Permission(String)` - 权限不足
- `Io(std::io::Error)` - IO 错误
- `Sdk(String)` - OSS SDK 错误
- `InvalidParameter(String)` - 无效参数
- `Timeout(String)` - 操作超时

## 示例

查看 `examples/` 目录中的完整示例：

- [`basic_usage.rs`](examples/basic_usage.rs) - 基本文件操作示例
- [`signed_url.rs`](examples/signed_url.rs) - 签名 URL 使用示例
- [`delete_file_demo.rs`](examples/delete_file_demo.rs) - 删除文件功能演示示例
- [`public_bucket_usage.rs`](examples/public_bucket_usage.rs) - 公有Bucket使用示例

运行示例：

```bash
# 设置环境变量
export OSS_ACCESS_KEY_ID="your_access_key_id"
export OSS_ACCESS_KEY_SECRET="your_access_key_secret"

# 运行基本使用示例
cargo run --example basic_usage

# 运行签名 URL 示例
cargo run --example signed_url

# 运行删除文件演示示例
cargo run --example delete_file_demo

# 运行公有Bucket使用示例
cargo run --example public_bucket_usage
```

## 使用示例

### 基本使用

```rust
use oss_client::{OssClient, PublicOssClient, OssConfig};

// 创建配置
let config = OssConfig::new("key_id".to_string(), "key_secret".to_string());

// 创建客户端
let private_client = OssClient::new(config.clone())?;
let public_client = PublicOssClient::new(config.clone())?;

// 使用私有bucket客户端
let upload_url = private_client.generate_upload_url(
    "documents/file.pdf", 
    Duration::from_secs(3600), 
    Some("application/pdf")
)?;

// 使用公有bucket客户端
let public_url = public_client.generate_public_download_url("documents/file.pdf")?;
```

### 使用统一接口 (OssClientTrait)

```rust
use oss_client::{OssClient, PublicOssClient, OssClientTrait, OssConfig};

// 创建客户端
let private_client = OssClient::new(config.clone())?;
let public_client = PublicOssClient::new(config.clone())?;

// 通过统一接口使用
let clients: Vec<&dyn OssClientTrait> = vec![&private_client, &public_client];

for client in clients {
    // 所有客户端都实现了相同的接口
    let object_key = client.generate_object_key("documents", Some("file.pdf"));
    let upload_url = client.generate_upload_url(
        &object_key, 
        Duration::from_secs(3600), 
        Some("application/pdf")
    )?;
    
    println!("对象键: {}, 上传URL: {}", object_key, upload_url);
}
```

### 通用函数示例

```rust
// 通用的文件上传函数，接受任何实现了 OssClientTrait 的客户端
async fn upload_file_generic(
    client: &dyn OssClientTrait,
    local_path: &str,
    object_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    client.upload_file(local_path, object_key).await
}

// 可以用于任何类型的客户端
let url1 = upload_file_generic(&private_client, "/local/file.pdf", "documents/file.pdf").await?;
let url2 = upload_file_generic(&public_client, "/local/image.jpg", "images/image.jpg").await?;
```

## 开发

### 构建

```