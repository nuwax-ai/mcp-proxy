# OSS Client

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# OSS Client

A lightweight and easy-to-use Alibaba Cloud OSS (Object Storage Service) client library, providing basic file operations and signed URL functionality.

## Features

### OssClientTrait (Unified Interface)
- **Unified Interface**: Defines basic operation interfaces for OSS clients
- **Polymorphic Support**: Supports unified usage of private bucket and public bucket clients
- **Code Reuse**: Reduces duplicate code, improves maintainability

### OssClient (Private Bucket Client)
- **File Operations**: Upload, download, delete files
- **Signed URLs**: Generate upload/download signed URLs with expiration time
- **File Management**: Check file existence, generate unique object keys
- **Connection Test**: Test OSS connection status

### PublicOssClient (Public Bucket Client)
- **Public Access**: Generate unsigned public download/access URLs
- **Batch Operations**: Batch generate public URLs
- **File Operations**: Upload, download, delete files (using public bucket)
- **Signed URLs**: Generate upload signed URLs (using public bucket)
- **File Management**: Check file existence, generate unique object keys
- **Connection Test**: Test public bucket connection status
- **Metadata Retrieval**: Get file metadata (via HTTP HEAD requests)

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
oss-client = { path = "../oss-client" }  # If in the same workspace
# or
oss-client = "0.1.0"  # If published to crates.io
```

## Quick Start

### Environment Variable Configuration

```bash
export OSS_ACCESS_KEY_ID="your_access_key_id"
export OSS_ACCESS_KEY_SECRET="your_access_key_secret"
```

### Basic Usage

```rust
use oss_client::{OssClient, OssConfig};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client from environment variables (recommended)
    let client = oss_client::create_client_from_env()?;

    // Upload file
    let upload_url = client.upload_file("local/document.pdf", "uploads/document.pdf")?;
    println!("File uploaded successfully: {}", upload_url);

    // Check if file exists
    let exists = client.file_exists("uploads/document.pdf")?;
    println!("File exists: {}", exists);

    // Download file
    client.download_file("uploads/document.pdf", "downloaded/document.pdf")?;
    println!("File downloaded successfully");

    Ok(())
}
```

## License

MIT OR Apache-2.0

## Contributing

Issues and Pull Requests are welcome!
