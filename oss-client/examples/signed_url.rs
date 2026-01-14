//! 签名URL使用示例

use oss_client::{OssClientTrait, PrivateOssClient};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OSS签名URL使用示例 ===\n");

    // 从环境变量创建客户端

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
            println!("✓ OSS客户端创建成功");
            println!("  基础URL: {}\n", client.get_base_url());

            // 演示签名URL生成
            demonstrate_signed_urls(&client)?;
        }
        Err(e) => {
            println!("✗ 创建OSS客户端失败: {e}");
            println!("请设置以下环境变量：");
            println!("  export OSS_ACCESS_KEY_ID=\"your_access_key_id\"");
            println!("  export OSS_ACCESS_KEY_SECRET=\"your_access_key_secret\"");
            println!("\n使用演示配置继续...\n");

            // 使用演示配置（显式提供全部参数）
            let config = oss_client::OssConfig::new(
                oss_client::defaults::ENDPOINT.to_string(),
                oss_client::defaults::PUBLIC_BUCKET.to_string(),
                "demo_access_key_id".to_string(),
                "demo_access_key_secret".to_string(),
                oss_client::defaults::REGION.to_string(),
                oss_client::defaults::UPLOAD_DIRECTORY.to_string(),
            );
            let client = PrivateOssClient::new(config)?;
            demonstrate_signed_urls(&client)?;
        }
    }

    Ok(())
}

fn demonstrate_signed_urls(client: &dyn OssClientTrait) -> Result<(), Box<dyn std::error::Error>> {
    println!("=== 签名URL生成演示 ===");

    // 生成上传签名URL
    println!("1. 生成上传签名URL");
    let object_key = "uploads/document.pdf";
    let expires_in = Duration::from_secs(4 * 3600); // 4小时有效

    println!("  对象键: {object_key}");
    println!("  有效期: {} 小时", expires_in.as_secs() / 3600);
    println!("  内容类型: application/pdf");

    match client.generate_upload_url(object_key, expires_in, Some("application/pdf")) {
        Ok(upload_url) => {
            println!("  ✓ 上传URL生成成功:");
            println!("    {upload_url}");
            println!("  使用方法:");
            println!("    curl -X PUT \"{upload_url}\" \\");
            println!("         -H \"Content-Type: application/pdf\" \\");
            println!("         --data-binary @local_file.pdf");
        }
        Err(e) => {
            println!("  ✗ 上传URL生成失败: {e}");
            println!("  注意：需要有效的OSS凭证才能生成签名URL");
        }
    }

    println!();

    // 生成下载签名URL（4小时有效）
    println!("2. 生成下载签名URL（4小时有效）");
    match client.generate_download_url(object_key, Some(expires_in)) {
        Ok(download_url) => {
            println!("  ✓ 下载URL生成成功:");
            println!("    {download_url}");
            println!("  使用方法:");
            println!("    curl \"{download_url}\" -o downloaded_file.pdf");
        }
        Err(e) => {
            println!("  ✗ 下载URL生成失败: {e}");
        }
    }

    println!();

    // 生成永久下载URL（实际上是4小时有效，因为我们的实现中没有真正的永久URL）
    println!("3. 生成下载签名URL（默认1小时有效）");
    match client.generate_download_url(object_key, None) {
        Ok(download_url) => {
            println!("  ✓ 下载URL生成成功:");
            println!("    {download_url}");
        }
        Err(e) => {
            println!("  ✗ 下载URL生成失败: {e}");
        }
    }

    println!();

    // 演示不同文件类型的上传URL
    println!("4. 不同文件类型的上传URL");
    let file_examples = vec![
        ("uploads/image.jpg", "image/jpeg"),
        (
            "uploads/document.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        ("uploads/data.json", "application/json"),
        ("uploads/video.mp4", "video/mp4"),
    ];

    for (key, content_type) in file_examples {
        println!("  文件: {key} ({content_type})");
        match client.generate_upload_url(key, Duration::from_secs(3600), Some(content_type)) {
            Ok(url) => println!("    ✓ URL: {url}"),
            Err(e) => println!("    ✗ 失败: {e}"),
        }
    }

    println!("\n=== 使用提示 ===");
    println!("1. 上传签名URL允许客户端直接上传文件到OSS，无需暴露访问密钥");
    println!("2. 下载签名URL允许临时访问私有文件");
    println!("3. 签名URL有时效性，过期后需要重新生成");
    println!("4. 在生产环境中，建议根据实际需求设置合适的过期时间");

    Ok(())
}
