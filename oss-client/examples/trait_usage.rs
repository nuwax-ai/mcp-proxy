//! 展示如何使用统一的 OssClientTrait 接口
//!
//! 这个示例展示了如何通过 trait 接口统一使用私有bucket和公有bucket客户端

use oss_client::{OssClientTrait, OssConfig, PrivateOssClient, PublicOssClient};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OSS客户端统一接口使用示例 ===\n");

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

    println!("1. 通过统一接口使用私有bucket客户端:");
    demonstrate_client_usage(&private_client, "私有bucket").await?;

    println!("\n2. 通过统一接口使用公有bucket客户端:");
    demonstrate_client_usage(&public_client, "公有bucket").await?;

    println!("\n3. 多态使用示例:");
    demonstrate_polymorphic_usage(&private_client, &public_client).await?;

    Ok(())
}

/// 演示客户端的基本使用
async fn demonstrate_client_usage(
    client: &dyn OssClientTrait,
    client_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("  客户端类型: {client_type}");
    println!("  配置信息: {:?}", client.get_config());
    println!("  基础URL: {}", client.get_base_url());

    // 生成上传签名URL
    let expires_in = Duration::from_secs(3600); // 1小时
    match client.generate_upload_url("documents/test.pdf", expires_in, Some("application/pdf")) {
        Ok(url) => println!("  上传签名URL: {url}"),
        Err(e) => println!("  生成上传签名URL失败: {e}"),
    }

    // 生成下载签名URL
    match client.generate_download_url("documents/test.pdf", Some(expires_in)) {
        Ok(url) => println!("  下载签名URL: {url}"),
        Err(e) => println!("  生成下载签名URL失败: {e}"),
    }

    // 生成唯一对象键
    let object_key = client.generate_object_key("documents", Some("manual.pdf"));
    println!("  生成的对象键: {object_key}");

    Ok(())
}

/// 演示多态使用
async fn demonstrate_polymorphic_usage(
    private_client: &dyn OssClientTrait,
    public_client: &dyn OssClientTrait,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("  多态使用 - 可以统一处理不同类型的客户端");

    // 创建一个客户端列表
    let clients: Vec<&dyn OssClientTrait> = vec![private_client, public_client];

    for (i, client) in clients.iter().enumerate() {
        println!("  客户端 {}: {}", i + 1, client.get_base_url());

        // 所有客户端都实现了相同的接口
        let object_key = client.generate_object_key("test", Some("file.txt"));
        println!("    生成的对象键: {object_key}");

        // 测试连接（需要实际的OSS权限）
        println!("    测试连接...");
        match client.test_connection().await {
            Ok(_) => println!("    ✅ 连接成功"),
            Err(e) => println!("    ❌ 连接失败: {e}"),
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
    println!("  使用通用接口上传文件: {local_path} -> {object_key}");

    // 通过 trait 接口调用上传方法
    let url = client.upload_file(local_path, object_key).await?;
    println!("  上传成功，文件URL: {url}");

    Ok(url)
}

/// 通用的文件删除函数
async fn delete_file_generic(
    client: &dyn OssClientTrait,
    object_key: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("  使用通用接口删除文件: {object_key}");

    // 通过 trait 接口调用删除方法
    client.delete_file(object_key).await?;
    println!("  删除成功");

    Ok(())
}
