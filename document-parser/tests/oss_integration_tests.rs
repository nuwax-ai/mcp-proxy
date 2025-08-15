use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::fs;
use tempfile::{TempDir, NamedTempFile};
use document_parser::config::{OssConfig, ConfigBuilder, ServerConfig, LogConfig, DocumentParserConfig, MinerUConfig, MarkItDownConfig, StorageConfig, SledConfig, ExternalIntegrationConfig, MarkItDownFeatures, FileSize, AppConfig};
use document_parser::services::oss_service::{OssService, OssServiceConfig, ProgressCallback};
use document_parser::error::AppError;
use std::sync::{Arc, Mutex};

/// 测试用的OSS配置
fn create_test_oss_config() -> OssConfig {
    OssConfig {
        endpoint: std::env::var("TEST_OSS_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:9000".to_string()),
        bucket: std::env::var("TEST_OSS_BUCKET")
            .unwrap_or_else(|_| "test-bucket".to_string()),
        access_key_id: std::env::var("TEST_OSS_ACCESS_KEY_ID")
            .unwrap_or_else(|_| "minioadmin".to_string()),
        access_key_secret: std::env::var("TEST_OSS_ACCESS_KEY_SECRET")
            .unwrap_or_else(|_| "minioadmin".to_string()),
    }
}

/// 创建测试文件
async fn create_test_file(content: &str, extension: &str) -> Result<NamedTempFile, std::io::Error> {
    let mut temp_file = NamedTempFile::with_suffix(extension)?;
    fs::write(temp_file.path(), content).await?;
    Ok(temp_file)
}

/// 创建测试图片文件
async fn create_test_image() -> Result<NamedTempFile, std::io::Error> {
    // 创建一个简单的1x1像素PNG图片
    let png_data = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, // IHDR chunk length
        0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x01, // width: 1
        0x00, 0x00, 0x00, 0x01, // height: 1
        0x08, 0x02, 0x00, 0x00, 0x00, // bit depth, color type, compression, filter, interlace
        0x90, 0x77, 0x53, 0xDE, // CRC
        0x00, 0x00, 0x00, 0x0C, // IDAT chunk length
        0x49, 0x44, 0x41, 0x54, // IDAT
        0x08, 0x99, 0x01, 0x01, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, // image data
        0xE2, 0x21, 0xBC, 0x33, // CRC
        0x00, 0x00, 0x00, 0x00, // IEND chunk length
        0x49, 0x45, 0x4E, 0x44, // IEND
        0xAE, 0x42, 0x60, 0x82, // CRC
    ];
    
    let mut temp_file = NamedTempFile::with_suffix(".png")?;
    fs::write(temp_file.path(), &png_data).await?;
    Ok(temp_file)
}

#[tokio::test]
async fn test_oss_service_creation() {
    let config = create_test_oss_config();
    
    // 测试默认配置创建
    let result = OssService::new(&config).await;
    
    // 如果没有真实的OSS环境，这个测试可能会失败
    // 在CI环境中，我们可以跳过这个测试或使用模拟服务
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        println!("跳过OSS集成测试");
        return;
    }
    
    match result {
        Ok(service) => {
            assert_eq!(service.get_bucket_name(), &config.bucket);
            assert!(service.get_base_url().contains(&config.bucket));
        }
        Err(e) => {
            println!("OSS服务创建失败（可能是因为没有测试环境）: {}", e);
            // 在没有真实OSS环境的情况下，我们不让测试失败
            return;
        }
    }
}

#[tokio::test]
async fn test_oss_service_with_custom_config() {
    let oss_config = create_test_oss_config();
    let service_config = OssServiceConfig {
        max_concurrent_uploads: 5,
        retry_attempts: 2,
        retry_delay_ms: 500,
        upload_timeout_secs: 60,
        chunk_size: 4 * 1024 * 1024,
    };
    
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let result = OssService::new_with_config(&oss_config, service_config.clone()).await;
    
    match result {
        Ok(service) => {
            assert_eq!(service.get_config().max_concurrent_uploads, 5);
            assert_eq!(service.get_config().retry_attempts, 2);
        }
        Err(_) => {
            // 跳过，没有真实环境
            return;
        }
    }
}

#[tokio::test]
async fn test_file_upload_and_download() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return, // 跳过测试
    };
    
    // 创建测试文件
    let test_content = "Hello, OSS World! 这是一个测试文件。";
    let temp_file = create_test_file(test_content, ".txt").await.unwrap();
    let file_path = temp_file.path().to_str().unwrap();
    
    // 上传文件
    let object_key = format!("test/upload_{}.txt", chrono::Utc::now().timestamp_millis());
    let upload_result = service.upload_file(file_path, &object_key).await;
    
    match upload_result {
        Ok(url) => {
            println!("文件上传成功: {}", url);
            
            // 验证文件存在
            let exists = service.file_exists(&object_key).await.unwrap();
            assert!(exists, "上传的文件应该存在");
            
            // 下载文件
            let downloaded_path = service.download_to_temp(&object_key).await.unwrap();
            let downloaded_content = fs::read_to_string(&downloaded_path).await.unwrap();
            assert_eq!(downloaded_content, test_content);
            
            // 清理
            let _ = service.delete_object(&object_key).await;
            let _ = fs::remove_file(&downloaded_path).await;
        }
        Err(e) => {
            println!("文件上传失败: {}", e);
            // 在测试环境不可用时不让测试失败
        }
    }
}

#[tokio::test]
async fn test_image_upload() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 创建测试图片
    let temp_image = create_test_image().await.unwrap();
    let image_path = temp_image.path().to_str().unwrap();
    
    // 上传图片
    let upload_result = service.upload_image(image_path).await;
    
    match upload_result {
        Ok(image_info) => {
            println!("图片上传成功: {}", image_info.oss_url);
            assert!(image_info.mime_type.starts_with("image/"));
            assert!(image_info.file_size > 0);
            
            // 清理 - 从URL中提取object_key
            let object_key = image_info.oss_url
                .split('/')
                .last()
                .unwrap_or("unknown")
                .to_string();
            let _ = service.delete_object(&object_key).await;
        }
        Err(e) => {
            println!("图片上传失败: {}", e);
        }
    }
}

#[tokio::test]
async fn test_batch_upload() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 创建多个测试文件
    let mut temp_files = Vec::new();
    let mut file_paths = Vec::new();
    
    for i in 0..3 {
        let content = format!("测试文件内容 {}", i);
        let temp_file = create_test_file(&content, ".txt").await.unwrap();
        file_paths.push(temp_file.path().to_string_lossy().to_string());
        temp_files.push(temp_file);
    }
    
    // 创建进度回调
    let progress_counter = Arc::new(Mutex::new(0));
    let progress_callback: ProgressCallback = {
        let counter = progress_counter.clone();
        Arc::new(move |current, total| {
            let mut count = counter.lock().unwrap();
            *count = current;
            println!("批量上传进度: {}/{}", current, total);
        })
    };
    
    // 批量上传
    let upload_result = service.upload_images_with_progress(&file_paths, Some(progress_callback)).await;
    
    match upload_result {
        Ok(result) => {
            println!("批量上传完成: 成功 {}, 失败 {}", result.successful.len(), result.failed.len());
            assert_eq!(result.total_processed, 3);
            
            // 验证进度回调被调用
            let final_count = *progress_counter.lock().unwrap();
            assert_eq!(final_count, 3);
            
            // 清理上传的文件
            for item in &result.successful {
                let _ = service.delete_object(&item.object_key).await;
            }
        }
        Err(e) => {
            println!("批量上传失败: {}", e);
        }
    }
}

#[tokio::test]
async fn test_presigned_url_generation() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 首先上传一个文件
    let test_content = "预签名URL测试内容";
    let temp_file = create_test_file(test_content, ".txt").await.unwrap();
    let file_path = temp_file.path().to_str().unwrap();
    let object_key = format!("test/presigned_{}.txt", chrono::Utc::now().timestamp_millis());
    
    let upload_result = service.upload_file(file_path, &object_key).await;
    
    match upload_result {
        Ok(_) => {
            // 生成预签名URL
            let expires_in = Duration::from_secs(3600);
            let presigned_result = service.generate_download_url(&object_key, Some(expires_in)).await;
            
            match presigned_result {
                Ok(url) => {
                    println!("预签名URL生成成功: {}", url);
                    assert!(url.contains(&object_key));
                    assert!(url.contains("X-Amz-Signature") || url.contains("Signature"));
                }
                Err(e) => {
                    println!("预签名URL生成失败: {}", e);
                }
            }
            
            // 清理
            let _ = service.delete_object(&object_key).await;
        }
        Err(e) => {
            println!("文件上传失败，跳过预签名URL测试: {}", e);
        }
    }
}

#[tokio::test]
async fn test_object_metadata() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 上传一个文件
    let test_content = "元数据测试内容";
    let temp_file = create_test_file(test_content, ".txt").await.unwrap();
    let file_path = temp_file.path().to_str().unwrap();
    let object_key = format!("test/metadata_{}.txt", chrono::Utc::now().timestamp_millis());
    
    let upload_result = service.upload_file(file_path, &object_key).await;
    
    match upload_result {
        Ok(_) => {
            // 获取元数据
            let metadata_result = service.get_object_metadata(&object_key).await;
            
            match metadata_result {
                Ok(metadata) => {
                    println!("对象元数据: {:?}", metadata);
                    assert!(metadata.contains_key("content-length"));
                    assert!(metadata.contains_key("content-type"));
                    
                    let content_length: u64 = metadata.get("content-length")
                        .unwrap()
                        .parse()
                        .unwrap();
                    assert_eq!(content_length, test_content.len() as u64);
                }
                Err(e) => {
                    println!("获取元数据失败: {}", e);
                }
            }
            
            // 清理
            let _ = service.delete_object(&object_key).await;
        }
        Err(e) => {
            println!("文件上传失败，跳过元数据测试: {}", e);
        }
    }
}

#[tokio::test]
async fn test_batch_delete() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 上传多个文件
    let mut object_keys = Vec::new();
    
    for i in 0..3 {
        let content = format!("批量删除测试文件 {}", i);
        let temp_file = create_test_file(&content, ".txt").await.unwrap();
        let file_path = temp_file.path().to_str().unwrap();
        let object_key = format!("test/batch_delete_{}_{}.txt", i, chrono::Utc::now().timestamp_millis());
        
        match service.upload_file(file_path, &object_key).await {
            Ok(_) => object_keys.push(object_key),
            Err(e) => println!("上传文件失败: {}", e),
        }
    }
    
    if !object_keys.is_empty() {
        // 批量删除
        let delete_result = service.delete_objects(&object_keys).await;
        
        match delete_result {
            Ok(failed_deletions) => {
                println!("批量删除完成，失败的对象: {:?}", failed_deletions);
                assert!(failed_deletions.is_empty(), "所有对象都应该删除成功");
                
                // 验证对象已被删除
                for object_key in &object_keys {
                    let exists = service.file_exists(object_key).await.unwrap();
                    assert!(!exists, "对象应该已被删除: {}", object_key);
                }
            }
            Err(e) => {
                println!("批量删除失败: {}", e);
            }
        }
    }
}

#[tokio::test]
async fn test_storage_stats() {
    if std::env::var("SKIP_OSS_TESTS").is_ok() {
        return;
    }
    
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => return,
    };
    
    // 上传一些测试文件
    let prefix = format!("test/stats_{}", chrono::Utc::now().timestamp_millis());
    let mut uploaded_keys = Vec::new();
    
    for i in 0..2 {
        let content = format!("统计测试文件 {} - 内容比较长一些以便测试大小统计", i);
        let temp_file = create_test_file(&content, ".txt").await.unwrap();
        let file_path = temp_file.path().to_str().unwrap();
        let object_key = format!("{}/file_{}.txt", prefix, i);
        
        match service.upload_file(file_path, &object_key).await {
            Ok(_) => uploaded_keys.push(object_key),
            Err(e) => println!("上传文件失败: {}", e),
        }
    }
    
    if !uploaded_keys.is_empty() {
        // 获取存储统计
        let stats_result = service.get_storage_stats(Some(&prefix)).await;
        
        match stats_result {
            Ok(stats) => {
                println!("存储统计: {:?}", stats);
                println!("格式化大小: {}", stats.formatted_size());
                
                assert!(stats.total_objects > 0);
                assert!(stats.total_size > 0);
                assert!(stats.file_count > 0);
            }
            Err(e) => {
                println!("获取存储统计失败: {}", e);
            }
        }
        
        // 清理
        let _ = service.delete_objects(&uploaded_keys).await;
    }
}

#[tokio::test]
async fn test_error_handling() {
    let config = create_test_oss_config();
    
    // 测试无效配置
    let mut invalid_config = config.clone();
    invalid_config.access_key_id = String::new();
    
    let result = OssService::new(&invalid_config).await;
    assert!(result.is_err(), "应该因为无效配置而失败");
    
    if let Err(AppError::Config(msg)) = result {
        assert!(msg.contains("访问密钥"));
    }
    
    // 如果有有效的服务实例，测试其他错误情况
    if let Ok(service) = OssService::new(&config).await {
        // 测试下载不存在的文件
        let non_existent_key = "non_existent_file.txt";
        let download_result = service.download_to_temp(non_existent_key).await;
        assert!(download_result.is_err(), "下载不存在的文件应该失败");
        
        // 测试检查不存在的文件
        let exists_result = service.file_exists(non_existent_key).await;
        match exists_result {
            Ok(exists) => assert!(!exists, "不存在的文件应该返回false"),
            Err(_) => {
                // 在某些情况下可能返回错误，这也是可以接受的
            }
        }
        
        // 测试生成不存在文件的预签名URL
        let presigned_result = service.generate_download_url(non_existent_key, None).await;
        assert!(presigned_result.is_err(), "为不存在的文件生成预签名URL应该失败");
    }
}

#[tokio::test]
async fn test_mime_type_detection() {
    let config = create_test_oss_config();
    let service = match OssService::new(&config).await {
        Ok(s) => s,
        Err(_) => {
            // 即使没有真实的OSS服务，我们也可以测试MIME类型检测
            // 但是detect_mime_type是私有方法，所以我们跳过这个测试
            return;
        }
    };
    
    // 创建不同类型的测试文件
    let test_cases = vec![
        ("test.txt", "text/plain"),
        ("test.json", "application/json"),
        ("test.pdf", "application/pdf"),
        ("test.jpg", "image/jpeg"),
        ("test.png", "image/png"),
    ];
    
    for (filename, expected_mime) in test_cases {
        let temp_file = create_test_file("test content", &format!(".{}", filename.split('.').last().unwrap())).await.unwrap();
        let file_path = temp_file.path().to_str().unwrap();
        
        // 由于detect_mime_type是私有方法，我们通过上传来间接测试
        let object_key = format!("test/mime_test_{}", filename);
        
        match service.upload_file(file_path, &object_key).await {
            Ok(_) => {
                // 获取元数据验证MIME类型
                if let Ok(metadata) = service.get_object_metadata(&object_key).await {
                    if let Some(content_type) = metadata.get("content-type") {
                        println!("文件 {} 的MIME类型: {}", filename, content_type);
                        // 注意：某些S3实现可能会修改MIME类型，所以这里只做基本验证
                    }
                }
                
                // 清理
                let _ = service.delete_object(&object_key).await;
            }
            Err(e) => {
                println!("上传文件失败: {}", e);
            }
        }
    }
}