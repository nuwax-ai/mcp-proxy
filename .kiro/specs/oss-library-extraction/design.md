# OSS库抽取设计文档

## 概述

设计一个简洁的Rust OSS库，从现有的document-parser项目中抽取OSS相关功能，提供基本的阿里云OSS操作能力。该库将作为独立的workspace成员，供其他项目复用。

## 架构

### 项目结构
```
oss-client/
├── Cargo.toml
├── src/
│   ├── lib.rs          # 库入口和公共API
│   ├── client.rs       # OSS客户端实现
│   ├── config.rs       # 配置结构体
│   ├── error.rs        # 错误类型定义
│   └── utils.rs        # 工具函数（MIME类型检测等）
├── examples/
│   ├── basic_usage.rs  # 基本使用示例
│   └── signed_url.rs   # 签名URL示例
└── README.md
```

### 依赖关系
- 基于 `aliyun-oss-rust-sdk` 进行OSS操作
- 使用 `serde` 进行配置序列化
- 使用 `thiserror` 进行错误处理
- 使用 `uuid` 生成唯一文件名

## 组件和接口

### 1. 配置结构体 (config.rs)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OssConfig {
    pub endpoint: String,
    pub bucket: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    pub region: String,
    pub upload_directory: String,
}

impl OssConfig {
    /// 从环境变量创建配置（使用默认值）
    pub fn from_env() -> Result<Self, OssError> {
        let access_key_id = std::env::var("OSS_ACCESS_KEY_ID")
            .map_err(|_| OssError::Config("环境变量 OSS_ACCESS_KEY_ID 未设置".to_string()))?;
        let access_key_secret = std::env::var("OSS_ACCESS_KEY_SECRET")
            .map_err(|_| OssError::Config("环境变量 OSS_ACCESS_KEY_SECRET 未设置".to_string()))?;
        
        Ok(Self {
            endpoint: "oss-rg-china-mainland.aliyuncs.com".to_string(),
            bucket: "nuwa-packages".to_string(),
            access_key_id,
            access_key_secret,
            region: "oss-rg-china-mainland".to_string(),
            upload_directory: "edu".to_string(),
        })
    }
    
    /// 创建自定义配置（只需要提供access keys，其他使用默认值）
    pub fn new(access_key_id: String, access_key_secret: String) -> Self {
        Self {
            endpoint: "oss-rg-china-mainland.aliyuncs.com".to_string(),
            bucket: "nuwa-packages".to_string(),
            access_key_id,
            access_key_secret,
            region: "oss-rg-china-mainland".to_string(),
            upload_directory: "edu".to_string(),
        }
    }
    
    /// 验证配置有效性
    pub fn validate(&self) -> Result<(), OssError> {
        if self.access_key_id.is_empty() {
            return Err(OssError::Config("access_key_id 不能为空".to_string()));
        }
        if self.access_key_secret.is_empty() {
            return Err(OssError::Config("access_key_secret 不能为空".to_string()));
        }
        Ok(())
    }
}
```

### 2. 错误类型 (error.rs)

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OssError {
    #[error("配置错误: {0}")]
    Config(String),
    
    #[error("网络错误: {0}")]
    Network(String),
    
    #[error("文件不存在: {0}")]
    FileNotFound(String),
    
    #[error("权限不足: {0}")]
    Permission(String),
    
    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("OSS SDK错误: {0}")]
    Sdk(String),
}

pub type Result<T> = std::result::Result<T, OssError>;
```

### 3. OSS客户端 (client.rs)

```rust
use aliyun_oss_rust_sdk::oss::OSS;
use std::time::Duration;
use std::path::Path;

pub struct OssClient {
    client: OSS,
    config: OssConfig,
}

impl OssClient {
    /// 创建新的OSS客户端
    pub fn new(config: OssConfig) -> Result<Self> {
        config.validate()?;
        
        let client = OSS::new(
            &config.access_key_id,
            &config.access_key_secret,
            &config.endpoint,
            &config.bucket,
        );
        
        Ok(Self { client, config })
    }
    
    /// 上传文件
    pub fn upload_file(&self, local_path: &str, object_key: &str) -> Result<String> {
        // 实现文件上传，返回OSS URL
    }
    
    /// 上传内容
    pub fn upload_content(&self, content: &[u8], object_key: &str, content_type: Option<&str>) -> Result<String> {
        // 实现内容上传，返回OSS URL
    }
    
    /// 下载文件
    pub fn download_file(&self, object_key: &str, local_path: &str) -> Result<()> {
        // 实现文件下载
    }
    
    /// 删除文件
    pub fn delete_file(&self, object_key: &str) -> Result<()> {
        // 实现文件删除
    }
    
    /// 检查文件是否存在
    pub fn file_exists(&self, object_key: &str) -> Result<bool> {
        // 检查文件存在性
    }
    
    /// 生成上传签名URL
    pub fn generate_upload_url(&self, object_key: &str, expires_in: Duration, content_type: Option<&str>) -> Result<String> {
        // 生成预签名上传URL
    }
    
    /// 生成下载签名URL
    pub fn generate_download_url(&self, object_key: &str, expires_in: Option<Duration>) -> Result<String> {
        // 生成预签名下载URL，None表示永久有效
    }
    
    /// 生成唯一的object key
    pub fn generate_object_key(&self, prefix: &str, filename: Option<&str>) -> String {
        // 生成带时间戳和UUID的唯一文件名
    }
}
```

### 4. 工具函数 (utils.rs)

```rust
/// 检测文件MIME类型
pub fn detect_mime_type(file_path: &str) -> String {
    // 根据文件扩展名返回MIME类型
}

/// 清理文件名
pub fn sanitize_filename(filename: &str) -> String {
    // 移除特殊字符，保留安全的文件名
}

/// 格式化文件大小
pub fn format_file_size(size: u64) -> String {
    // 将字节数格式化为可读的大小字符串
}
```

### 5. 库入口 (lib.rs)

```rust
//! 简洁的阿里云OSS操作库
//! 
//! 提供基本的文件上传、下载、删除功能，以及预签名URL生成。

pub mod client;
pub mod config;
pub mod error;
pub mod utils;

// 重新导出主要类型
pub use client::OssClient;
pub use config::OssConfig;
pub use error::{OssError, Result};

// 便捷函数
pub fn create_client(config: OssConfig) -> Result<OssClient> {
    OssClient::new(config)
}

pub fn create_client_from_env() -> Result<OssClient> {
    let config = OssConfig::from_env()?;
    OssClient::new(config)
}
```

## 数据模型

### 配置模型
```rust
#[derive(Debug, Clone)]
pub struct OssConfig {
    pub endpoint: String,           // OSS endpoint (默认: oss-rg-china-mainland.aliyuncs.com)
    pub bucket: String,             // 存储桶名称 (默认: nuwa-packages)
    pub access_key_id: String,      // 访问密钥ID (必须通过环境变量设置)
    pub access_key_secret: String,  // 访问密钥Secret (必须通过环境变量设置)
    pub region: String,             // 区域 (默认: oss-rg-china-mainland)
    pub upload_directory: String,   // 上传目录前缀 (默认: edu)
}
```

### 响应模型
```rust
#[derive(Debug, Clone)]
pub struct UploadResult {
    pub object_key: String,    // OSS对象键
    pub url: String,           // 访问URL
    pub size: u64,             // 文件大小
}

#[derive(Debug, Clone)]
pub struct SignedUrl {
    pub url: String,           // 签名URL
    pub expires_at: Option<String>, // 过期时间
}
```

## 错误处理

### 错误分类
1. **配置错误** - 配置参数无效或缺失
2. **网络错误** - 网络连接问题
3. **文件错误** - 文件不存在或IO错误
4. **权限错误** - OSS访问权限不足
5. **SDK错误** - 底层SDK返回的错误

### 错误处理策略
- 所有公共API返回 `Result<T, OssError>`
- 提供详细的错误信息和上下文
- 不进行自动重试，由调用方决定重试策略
- 错误信息支持中文，便于调试

## 测试策略

### 单元测试
- 配置验证测试
- 文件名清理测试
- MIME类型检测测试
- 错误处理测试

### 集成测试
- 实际OSS操作测试（需要测试环境）
- 签名URL生成和使用测试
- 配置加载测试

### 示例代码
提供完整的使用示例，包括：
- 基本文件上传下载
- 签名URL生成和使用
- 错误处理最佳实践

## 部署和集成

### Cargo.toml配置
```toml
[package]
name = "oss-client"
version = "0.1.0"
edition = "2024"
description = "简洁的阿里云OSS操作库"
license = "MIT"

[dependencies]
aliyun-oss-rust-sdk = "0.2.1"
serde = { version = "1.0", features = ["derive"] }
thiserror = "1.0"
uuid = { version = "1.0", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tokio = { version = "1.0", features = ["full"] }
```

### 集成到现有项目
1. 在workspace的Cargo.toml中添加oss-client成员
2. 在需要使用的项目中添加依赖：`oss-client = { path = "../oss-client" }`
3. 替换现有的OSS相关代码

### 环境变量配置
```bash
# 必须设置的环境变量
export OSS_ACCESS_KEY_ID="your_access_key_id"
export OSS_ACCESS_KEY_SECRET="your_access_key_secret"

# 其他配置使用默认值：
# endpoint: "oss-rg-china-mainland.aliyuncs.com"
# bucket: "nuwa-packages"
# region: "oss-rg-china-mainland"
# upload_directory: "edu"
```

## 使用示例

### 基本使用
```rust
use oss_client::{OssClient, OssConfig};

// 方式1：从环境变量创建客户端（使用默认配置）
let client = oss_client::create_client_from_env()?;

// 方式2：手动创建配置
let config = OssConfig::new(
    "your_access_key_id".to_string(),
    "your_access_key_secret".to_string()
);
let client = OssClient::new(config)?;

// 上传文件
let url = client.upload_file("local/file.txt", "remote/file.txt")?;
println!("文件上传成功: {}", url);

// 生成上传签名URL
let upload_url = client.generate_upload_url(
    "uploads/document.pdf",
    Duration::from_secs(4 * 3600), // 4小时有效
    Some("application/pdf")
)?;
println!("上传URL: {}", upload_url);
```

### 签名URL使用
```rust
// 生成下载签名URL（4小时有效）
let download_url = client.generate_download_url(
    "uploads/document.pdf",
    Some(Duration::from_secs(4 * 3600))
)?;

// 生成永久下载URL
let permanent_url = client.generate_download_url(
    "uploads/document.pdf",
    None
)?;
```

这个设计保持了简洁性，专注于核心功能，易于理解和使用。