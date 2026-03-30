//! 签名URL使用示例

use oss_client::{OssClientTrait, PrivateOssClient};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Example of using OSS signed URL ===\\n");

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
            println!("✓ OSS client created successfully");
            println!("Base URL: {}\\n", client.get_base_url());

            // 演示签名URL生成
            demonstrate_signed_urls(&client)?;
        }
        Err(e) => {
            println!("✗ Failed to create OSS client: {e}");
            println!("Please set the following environment variables:");
            println!("  export OSS_ACCESS_KEY_ID=\"your_access_key_id\"");
            println!("  export OSS_ACCESS_KEY_SECRET=\"your_access_key_secret\"");
            println!("\\nContinue with demo configuration...\\n");

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
    println!("=== Signed URL generation demo ===");

    // 生成上传签名URL
    println!("1. Generate upload signature URL");
    let object_key = "uploads/document.pdf";
    let expires_in = Duration::from_secs(4 * 3600); // 4小时有效

    println!("Object key: {object_key}");
    println!("Validity period: {} hours", expires_in.as_secs() / 3600);
    println!("Content type: application/pdf");

    match client.generate_upload_url(object_key, expires_in, Some("application/pdf")) {
        Ok(upload_url) => {
            println!("✓ Upload URL generated successfully:");
            println!("    {upload_url}");
            println!("How to use:");
            println!("    curl -X PUT \"{upload_url}\" \\");
            println!("         -H \"Content-Type: application/pdf\" \\");
            println!("         --data-binary @local_file.pdf");
        }
        Err(e) => {
            println!("✗ Failed to generate upload URL: {e}");
            println!("NOTE: Valid OSS credentials are required to generate signed URLs");
        }
    }

    println!();

    // 生成下载签名URL（4小时有效）
    println!("2. Generate download signature URL (valid for 4 hours)");
    match client.generate_download_url(object_key, Some(expires_in)) {
        Ok(download_url) => {
            println!("✓ Download URL generated successfully:");
            println!("    {download_url}");
            println!("How to use:");
            println!("    curl \"{download_url}\" -o downloaded_file.pdf");
        }
        Err(e) => {
            println!("✗ Download URL generation failed: {e}");
        }
    }

    println!();

    // 生成永久下载URL（实际上是4小时有效，因为我们的实现中没有真正的永久URL）
    println!("3. Generate download signature URL (valid for 1 hour by default)");
    match client.generate_download_url(object_key, None) {
        Ok(download_url) => {
            println!("✓ Download URL generated successfully:");
            println!("    {download_url}");
        }
        Err(e) => {
            println!("✗ Download URL generation failed: {e}");
        }
    }

    println!();

    // 演示不同文件类型的上传URL
    println!("4. Upload URLs for different file types");
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
        println!("File: {key} ({content_type})");
        match client.generate_upload_url(key, Duration::from_secs(3600), Some(content_type)) {
            Ok(url) => println!("    ✓ URL: {url}"),
            Err(e) => println!("✗ Failure: {e}"),
        }
    }

    println!("\\n=== Usage Tips ===");
    println!(
        "1. Uploading a signed URL allows the client to directly upload files to OSS without exposing the access key."
    );
    println!("2. Downloading signed URLs allows temporary access to private files");
    println!("3. The signed URL is time-sensitive and needs to be regenerated after expiration.");
    println!(
        "4. In a production environment, it is recommended to set an appropriate expiration time based on actual needs."
    );

    Ok(())
}
