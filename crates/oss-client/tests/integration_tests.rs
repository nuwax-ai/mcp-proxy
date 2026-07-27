//! 集成测试
//!
//! 注意：这些测试需要有效的OSS凭证才能运行
//! 设置环境变量 OSS_ACCESS_KEY_ID 和 OSS_ACCESS_KEY_SECRET 来运行实际的OSS操作测试

use oss_client::{
    OssClientTrait, OssConfig, OssError, PrivateOssClient, PublicOssClient, defaults,
};
use std::time::Duration;

fn create_private_client_from_env() -> Result<PrivateOssClient, OssError> {
    let access_key_id = std::env::var("OSS_ACCESS_KEY_ID").unwrap_or_default();
    let access_key_secret = std::env::var("OSS_ACCESS_KEY_SECRET").unwrap_or_default();
    if access_key_id.is_empty() || access_key_secret.is_empty() {
        return Err(OssError::config("缺少OSS凭证环境变量"));
    }
    let bucket =
        std::env::var("OSS_BUCKET").unwrap_or_else(|_| defaults::PRIVATE_BUCKET.to_string());
    let endpoint = std::env::var("OSS_ENDPOINT").unwrap_or_else(|_| defaults::ENDPOINT.to_string());
    let region = std::env::var("OSS_REGION").unwrap_or_else(|_| defaults::REGION.to_string());
    let upload_directory =
        std::env::var("OSS_UPLOAD_DIR").unwrap_or_else(|_| defaults::UPLOAD_DIRECTORY.to_string());
    let config = OssConfig::new(
        endpoint,
        bucket,
        access_key_id,
        access_key_secret,
        region,
        upload_directory,
    );
    PrivateOssClient::new(config)
}

fn create_public_client_from_env() -> Result<PublicOssClient, OssError> {
    let access_key_id = std::env::var("OSS_ACCESS_KEY_ID").unwrap_or_default();
    let access_key_secret = std::env::var("OSS_ACCESS_KEY_SECRET").unwrap_or_default();
    if access_key_id.is_empty() || access_key_secret.is_empty() {
        return Err(OssError::config("缺少OSS凭证环境变量"));
    }
    let bucket =
        std::env::var("OSS_PUBLIC_BUCKET").unwrap_or_else(|_| defaults::PUBLIC_BUCKET.to_string());
    let endpoint = std::env::var("OSS_ENDPOINT").unwrap_or_else(|_| defaults::ENDPOINT.to_string());
    let region = std::env::var("OSS_REGION").unwrap_or_else(|_| defaults::REGION.to_string());
    let upload_directory =
        std::env::var("OSS_UPLOAD_DIR").unwrap_or_else(|_| defaults::UPLOAD_DIRECTORY.to_string());
    let config = OssConfig::new(
        endpoint,
        bucket,
        access_key_id,
        access_key_secret,
        region,
        upload_directory,
    );
    PublicOssClient::new(config)
}

/// 测试客户端创建
#[test]
fn test_client_creation() {
    // 测试从环境变量创建私有客户端
    match create_private_client_from_env() {
        Ok(client) => {
            let config = client.get_config();
            assert!(!config.access_key_id.is_empty());
            assert!(!config.access_key_secret.is_empty());
            assert!(!config.endpoint.is_empty());
            assert!(!config.bucket.is_empty());
        }
        Err(e) => {
            assert!(e.is_config_error());
        }
    }

    // 测试从环境变量创建公有客户端
    match create_public_client_from_env() {
        Ok(client) => {
            let config = client.get_config();
            assert!(!config.access_key_id.is_empty());
            assert!(!config.access_key_secret.is_empty());
            assert!(!config.endpoint.is_empty());
            assert!(!config.bucket.is_empty());
        }
        Err(e) => {
            assert!(e.is_config_error());
        }
    }

    // 测试使用自定义配置创建公有客户端
    let config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PUBLIC_BUCKET.to_string(),
        "test_key_id".to_string(),
        "test_key_secret".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let client = PublicOssClient::new(config).unwrap();
    assert_eq!(client.get_config().access_key_id, "test_key_id");
    assert_eq!(client.get_config().access_key_secret, "test_key_secret");
}

/// 测试配置验证
#[test]
fn test_config_validation() {
    // 测试空的access_key_id
    let config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PUBLIC_BUCKET.to_string(),
        "".to_string(),
        "secret".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let result = PublicOssClient::new(config);
    assert!(result.is_err());
    assert!(result.unwrap_err().is_config_error());

    // 测试空的access_key_secret
    let config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PUBLIC_BUCKET.to_string(),
        "key_id".to_string(),
        "".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let result = PublicOssClient::new(config);
    assert!(result.is_err());
    assert!(result.unwrap_err().is_config_error());
}

/// 测试签名URL生成（不需要实际OSS连接）
#[test]
fn test_signed_url_generation() {
    // 私有客户端：签名上传/下载链接
    let private_config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PRIVATE_BUCKET.to_string(),
        "test_key_id".to_string(),
        "test_key_secret".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let private_client = PrivateOssClient::new(private_config).unwrap();

    let upload_url = private_client.generate_upload_url(
        "test/file.txt",
        Duration::from_secs(3600),
        Some("text/plain"),
    );
    assert!(upload_url.is_ok());
    let url = upload_url.unwrap();
    assert!(
        url.contains("nuwa-packages.oss-rg-china-mainland.aliyuncs.com")
            || url.contains("edu-nuwa-packages.oss-rg-china-mainland.aliyuncs.com")
    );
    assert!(url.contains("edu/test/file.txt"));
    assert!(url.contains("Expires=") || url.contains("x-oss-expires"));
    assert!(url.contains("Signature=") || url.contains("x-oss-signature"));

    let download_url =
        private_client.generate_download_url("test/file.txt", Some(Duration::from_secs(3600)));
    assert!(download_url.is_ok());
    let url = download_url.unwrap();
    assert!(url.contains("edu/test/file.txt"));
    assert!(url.contains("Expires=") || url.contains("x-oss-expires"));
    assert!(url.contains("Signature=") || url.contains("x-oss-signature"));

    // 公有客户端：下载链接不应包含签名
    let public_config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PUBLIC_BUCKET.to_string(),
        "test_key_id".to_string(),
        "test_key_secret".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let public_client = PublicOssClient::new(public_config.clone()).unwrap();
    let public_download_url = public_client.generate_download_url("test/file.txt", None);
    let url = public_download_url.unwrap();
    // 验证URL包含正确的路径，但域名可能被替换为自定义域名
    assert!(url.contains("edu/test/file.txt"));
    // 由于 replace_oss_domain 可能替换域名，我们只验证路径部分
    assert!(!url.contains("Expires="));
    assert!(!url.contains("Signature="));
}

/// 测试object key生成
#[test]
fn test_object_key_generation() {
    let config = OssConfig::new(
        defaults::ENDPOINT.to_string(),
        defaults::PUBLIC_BUCKET.to_string(),
        "test_key_id".to_string(),
        "test_key_secret".to_string(),
        defaults::REGION.to_string(),
        defaults::UPLOAD_DIRECTORY.to_string(),
    );
    let client = PublicOssClient::new(config).unwrap();

    // 测试带文件名的object key生成
    let key1 = client.generate_object_key("uploads", Some("document.pdf"));
    assert!(key1.starts_with("uploads/"));
    assert!(key1.contains("document"));
    assert!(key1.ends_with(".pdf"));

    // 测试不带文件名的object key生成
    let key2 = client.generate_object_key("temp", None);
    assert!(key2.starts_with("temp/"));
    assert!(key2.contains("file_"));

    // 确保生成的key是唯一的
    let key3 = client.generate_object_key("uploads", Some("document.pdf"));
    assert_ne!(key1, key3);
}

/// 测试错误处理
#[test]
fn test_error_handling() {
    // 测试配置错误
    let config_err = OssError::config("test config error");
    assert!(config_err.is_config_error());
    assert!(!config_err.is_network_error());

    // 测试网络错误
    let network_err = OssError::network("test network error");
    assert!(network_err.is_network_error());
    assert!(!network_err.is_config_error());

    // 测试文件不存在错误
    let file_err = OssError::file_not_found("test.txt");
    assert!(file_err.is_file_not_found());

    // 测试权限错误
    let perm_err = OssError::permission("access denied");
    assert!(perm_err.is_permission_error());
}

// 以下测试需要有效的OSS凭证，只有在设置了环境变量时才会运行

#[tokio::test]
async fn test_actual_oss_operations() -> oss_client::Result<()> {
    // 只有在设置了环境变量时才运行
    let client = match create_private_client_from_env() {
        Ok(client) => client,
        Err(_) => {
            println!("Skip actual OSS operation test: environment variables not set");
            return Ok(());
        }
    };

    println!("Run actual OSS operation test...");

    // 测试文件存在性检查（对一个不存在的文件）
    let test_key = format!("test/non-existent-{}.txt", chrono::Utc::now().timestamp());
    match client.file_exists(&test_key).await {
        Ok(exists) => {
            assert!(!exists, "不存在的文件应该返回false");
            println!("✓ File existence check test passed");
        }
        Err(e) => {
            println!("File existence check failed: {e}");
        }
    }

    // 测试上传小文件
    let test_content = b"Hello, OSS!";
    let test_key = format!(
        "test/integration-test-{}.txt",
        chrono::Utc::now().timestamp()
    );

    match client
        .upload_content(test_content, &test_key, Some("text/plain"))
        .await
    {
        Ok(url) => {
            println!("✓ File uploaded successfully: {url}");

            // 测试文件存在性
            match client.file_exists(&test_key).await {
                Ok(exists) => {
                    assert!(exists, "上传的文件应该存在");
                    println!("✓ The file existence check passes after uploading");
                }
                Err(e) => println!("File existence check failed: {e}"),
            }

            // 清理测试文件
            match client.delete_file(&test_key).await {
                Ok(_) => println!("✓ Test file cleanup successful"),
                Err(e) => println!("Test file cleanup failed: {e}"),
            }
        }
        Err(e) => {
            println!("File upload failed: {e}");
            println!("This may be caused by invalid OSS credentials or insufficient permissions");
        }
    }

    Ok(())
}
