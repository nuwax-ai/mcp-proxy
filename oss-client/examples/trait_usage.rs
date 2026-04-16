//! 展示如何使用统一的 OssClientTrait 接口
//!
//! 这个示例展示了如何通过 trait 接口统一使用私有bucket和公有bucket客户端

use oss_client::{OssClientTrait, OssConfig, PrivateOssClient, PublicOssClient};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Example of using the OSS client unified interface ===\\n");

    // 创建配置
    let config = OssConfig::new(
        oss_client::defaults::ENDPOINT.to_string(),
        oss_client::defaults::PUBLIC_BUCKET.to_string(),
        "your_access_key_id".to_string(),
        "your_access_key_secret".to_string(),
        oss_client::defaults::REGION.to_string(),
        oss_client::defaults::UPLOAD_DIRECTORY.to_string(),
    );

    // 创建两个客户端
    let private_client = PrivateOssClient::new(config.clone())?;
    let public_client = PublicOssClient::new(config.clone())?;

    println!("1. Use private bucket client through unified interface:");
    demonstrate_client_usage(&private_client, "私有bucket").await?;

    println!("\\n2. Use the public bucket client through the unified interface:");
    demonstrate_client_usage(&public_client, "公有bucket").await?;

    println!("\\n3. Examples of polymorphic usage:");
    demonstrate_polymorphic_usage(&private_client, &public_client).await?;

    Ok(())
}

/// 演示客户端的基本使用
async fn demonstrate_client_usage(
    client: &dyn OssClientTrait,
    client_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Client type: {client_type}");
    println!("Configuration information: {:?}", client.get_config());
    println!("Base URL: {}", client.get_base_url());

    // 生成上传签名URL
    let expires_in = Duration::from_secs(3600); // 1小时
    match client.generate_upload_url("documents/test.pdf", expires_in, Some("application/pdf")) {
        Ok(url) => println!("Upload signature URL: {url}"),
        Err(e) => println!("Failed to generate upload signature URL: {e}"),
    }

    // 生成下载签名URL
    match client.generate_download_url("documents/test.pdf", Some(expires_in)) {
        Ok(url) => println!("Download signature URL: {url}"),
        Err(e) => println!("Failed to generate download signature URL: {e}"),
    }

    // 生成唯一对象键
    let object_key = client.generate_object_key("documents", Some("manual.pdf"));
    println!("Generated object key: {object_key}");

    Ok(())
}

/// 演示多态使用
async fn demonstrate_polymorphic_usage(
    private_client: &dyn OssClientTrait,
    public_client: &dyn OssClientTrait,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Polymorphic use - can handle different types of clients uniformly");

    // 创建一个客户端列表
    let clients: Vec<&dyn OssClientTrait> = vec![private_client, public_client];

    for (i, client) in clients.iter().enumerate() {
        println!("Client {}: {}", i + 1, client.get_base_url());

        // 所有客户端都实现了相同的接口
        let object_key = client.generate_object_key("test", Some("file.txt"));
        println!("Generated object key: {object_key}");

        // 测试连接（需要实际的OSS权限）
        println!("Test connection...");
        match client.test_connection().await {
            Ok(_) => println!("✅Connected successfully"),
            Err(e) => println!("❌ Connection failed: {e}"),
        }
    }

    Ok(())
}

/// 通用的文件操作函数，接受任何实现了 OssClientTrait 的客户端
async fn upload_file_generic(
    client: &dyn OssClientTrait,
    local_path: &str,
    object_key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    println!("Use the common interface to upload files: {local_path} -> {object_key}");

    // 通过 trait 接口调用上传方法
    let url = client.upload_file(local_path, object_key).await?;
    println!("Upload successful, file URL: {url}");

    Ok(url)
}

/// 通用的文件删除函数
async fn delete_file_generic(
    client: &dyn OssClientTrait,
    object_key: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("Delete files using common interface: {object_key}");

    // 通过 trait 接口调用删除方法
    client.delete_file(object_key).await?;
    println!("Delete successfully");

    Ok(())
}
