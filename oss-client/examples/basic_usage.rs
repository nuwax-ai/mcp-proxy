//! 基本使用示例

use oss_client::{OssConfig, PrivateOssClient, format_file_size, generate_random_filename};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Basic usage examples of OSS client ===\\n");

    // 显示库信息
    println!("Library name: {}", oss_client::name());
    println!("Version: {}", oss_client::version());
    println!("Description: {}\\n", oss_client::description());

    // 方式1：从环境变量创建客户端（推荐）
    println!("1. Try creating a client from environment variables...");
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
            println!("✓ Successful creation of client from environment variables");
            println!("Base URL: {}", client.get_base_url());

            // 演示工具函数
            demonstrate_utilities(&client)?;

            // 注意：实际的文件操作需要有效的OSS凭证
            println!(
                "\\nNote: To perform actual file operations, set the following environment variables:"
            );
            println!("  export OSS_ACCESS_KEY_ID=\"your_access_key_id\"");
            println!("  export OSS_ACCESS_KEY_SECRET=\"your_access_key_secret\"");
        }
        Err(e) => {
            println!("✗ Failed to create client from environment variable: {e}");
            println!(
                "Please set the OSS_ACCESS_KEY_ID and OSS_ACCESS_KEY_SECRET environment variables"
            );

            // 方式2：手动创建配置（演示用）
            println!("\\n2. Create a client using the sample configuration...");
            let config = OssConfig::new(
                oss_client::defaults::ENDPOINT.to_string(),
                oss_client::defaults::PUBLIC_BUCKET.to_string(),
                "demo_access_key_id".to_string(),
                "demo_access_key_secret".to_string(),
                oss_client::defaults::REGION.to_string(),
                oss_client::defaults::UPLOAD_DIRECTORY.to_string(),
            );
            let client = PrivateOssClient::new(config)?;
            println!("✓ Successful creation of client using sample configuration");
            println!("Base URL: {}", client.get_base_url());

            // 演示工具函数
            demonstrate_utilities(&client)?;
        }
    }

    Ok(())
}

fn demonstrate_utilities(client: &PrivateOssClient) -> Result<(), Box<dyn std::error::Error>> {
    println!("\\n=== Tool function demonstration ===");

    // 文件名处理
    let random_filename = generate_random_filename(Some("txt"));
    println!("Random file name: {random_filename}");

    // 文件大小格式化
    let sizes = vec![1024, 1024 * 1024, 1024 * 1024 * 1024];
    for size in sizes {
        println!("File size {} bytes = {}", size, format_file_size(size));
    }

    // MIME类型检测
    let files = vec!["test.jpg", "document.pdf", "data.xlsx", "video.mp4"];
    for file in files {
        println!(
            "MIME type of file {}: {}",
            file,
            oss_client::detect_mime_type(file)
        );
    }

    // 签名URL生成（演示，不会实际调用OSS）
    println!("\\n=== Signed URL function demonstration ===");
    println!("Note: The following features require valid OSS credentials to work properly");

    // 演示API调用（但不实际执行，因为可能没有有效凭证）
    println!("Available API methods:");
    println!("  - client.upload_file(local_path, object_key)");
    println!("  - client.upload_content(content, object_key, content_type)");
    println!("  - client.download_file(object_key, local_path)");
    println!("  - client.delete_file(object_key)");
    println!("  - client.file_exists(object_key)");
    println!("  - client.generate_upload_url(object_key, expires_in, content_type)");
    println!("  - client.generate_download_url(object_key, expires_in)");

    Ok(())
}
