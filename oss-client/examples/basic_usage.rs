//! 基本使用示例

use oss_client::{OssConfig, PrivateOssClient, format_file_size, generate_random_filename};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OSS客户端基本使用示例 ===\n");

    // 显示库信息
    println!("库名称: {}", oss_client::name());
    println!("版本: {}", oss_client::version());
    println!("描述: {}\n", oss_client::description());

    // 方式1：从环境变量创建客户端（推荐）
    println!("1. 尝试从环境变量创建客户端...");
    let oss_config = oss_client::OssConfig::new(
        oss_client::defaults::ENDPOINT.to_string(),
        oss_client::defaults::PUBLIC_BUCKET.to_string(),
        "demo_access_key_id".to_string(),
        "demo_access_key_secret".to_string(),
        oss_client::defaults::REGION.to_string(),
        oss_client::defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let client = PrivateOssClient::new(oss_config);
    match client {
        Ok(client) => {
            println!("✓ 从环境变量创建客户端成功");
            println!("  基础URL: {}", client.get_base_url());

            // 演示工具函数
            demonstrate_utilities(&client)?;

            // 注意：实际的文件操作需要有效的OSS凭证
            println!("\n注意：要进行实际的文件操作，请设置以下环境变量：");
            println!("  export OSS_ACCESS_KEY_ID=\"your_access_key_id\"");
            println!("  export OSS_ACCESS_KEY_SECRET=\"your_access_key_secret\"");
        }
        Err(e) => {
            println!("✗ 从环境变量创建客户端失败: {e}");
            println!("  请设置 OSS_ACCESS_KEY_ID 和 OSS_ACCESS_KEY_SECRET 环境变量");

            // 方式2：手动创建配置（演示用）
            println!("\n2. 使用示例配置创建客户端...");
            let config = OssConfig::new(
                oss_client::defaults::ENDPOINT.to_string(),
                oss_client::defaults::PUBLIC_BUCKET.to_string(),
                "demo_access_key_id".to_string(),
                "demo_access_key_secret".to_string(),
                oss_client::defaults::REGION.to_string(),
                oss_client::defaults::UPLOAD_DIRECTORY.to_string(),
            );
            let client = PrivateOssClient::new(config)?;
            println!("✓ 使用示例配置创建客户端成功");
            println!("  基础URL: {}", client.get_base_url());

            // 演示工具函数
            demonstrate_utilities(&client)?;
        }
    }

    Ok(())
}

fn demonstrate_utilities(client: &PrivateOssClient) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== 工具函数演示 ===");

    // 文件名处理
    let random_filename = generate_random_filename(Some("txt"));
    println!("随机文件名: {random_filename}");

    // 文件大小格式化
    let sizes = vec![1024, 1024 * 1024, 1024 * 1024 * 1024];
    for size in sizes {
        println!("文件大小 {} bytes = {}", size, format_file_size(size));
    }

    // MIME类型检测
    let files = vec!["test.jpg", "document.pdf", "data.xlsx", "video.mp4"];
    for file in files {
        println!(
            "文件 {} 的MIME类型: {}",
            file,
            oss_client::detect_mime_type(file)
        );
    }

    // 签名URL生成（演示，不会实际调用OSS）
    println!("\n=== 签名URL功能演示 ===");
    println!("注意：以下功能需要有效的OSS凭证才能正常工作");

    // 演示API调用（但不实际执行，因为可能没有有效凭证）
    println!("可用的API方法：");
    println!("  - client.upload_file(local_path, object_key)");
    println!("  - client.upload_content(content, object_key, content_type)");
    println!("  - client.download_file(object_key, local_path)");
    println!("  - client.delete_file(object_key)");
    println!("  - client.file_exists(object_key)");
    println!("  - client.generate_upload_url(object_key, expires_in, content_type)");
    println!("  - client.generate_download_url(object_key, expires_in)");

    Ok(())
}
