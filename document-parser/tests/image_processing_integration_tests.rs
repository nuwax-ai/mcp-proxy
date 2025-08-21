use document_parser::services::image_processor::{
    ImageProcessor, ImageProcessorConfig, ImageUploadResult,
};
use std::path::Path;
use tempfile::TempDir;

#[tokio::test]
async fn test_image_processing_pipeline() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 创建测试图片文件
    let test_images = create_test_images(&temp_dir).await;

    // 测试批量处理
    let result = processor.batch_upload_images(test_images.clone()).await;

    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert_eq!(batch_result.len(), test_images.len());

    // 检查结果
    for upload_result in &batch_result {
        // 由于没有OSS服务，预期会失败但不会panic
        assert!(!upload_result.success);
        assert!(upload_result.error_message.is_some());
    }
}

#[tokio::test]
async fn test_image_extraction_from_directory() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 创建测试目录结构
    let test_dir = temp_dir.path().join("images");
    tokio::fs::create_dir_all(&test_dir).await.unwrap();

    // 创建测试图片
    create_test_image(&test_dir.join("test1.jpg")).await;
    create_test_image(&test_dir.join("test2.png")).await;

    // 测试提取（注意：当前实现不支持递归子目录）
    let result = processor
        .extract_images_from_directory(test_dir.to_str().unwrap())
        .await;

    assert!(result.is_ok());
    let image_paths = result.unwrap();
    assert_eq!(image_paths.len(), 2);
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 测试不存在的文件
    let non_existent_files = vec![
        "/non/existent/file1.jpg".to_string(),
        "/non/existent/file2.png".to_string(),
    ];

    let result = processor.batch_upload_images(non_existent_files).await;

    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert_eq!(batch_result.len(), 2);

    // 所有结果都应该失败
    for upload_result in &batch_result {
        assert!(!upload_result.success);
        assert!(upload_result.error_message.is_some());
    }
}

#[tokio::test]
async fn test_performance_with_large_batch() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 创建大量测试图片
    let mut test_images = Vec::new();
    for i in 0..50 {
        let image_path = temp_dir.path().join(format!("test_{i}.jpg"));
        create_test_image(&image_path).await;
        test_images.push(image_path.to_string_lossy().to_string());
    }

    let start_time = std::time::Instant::now();

    let result = processor.batch_upload_images(test_images.clone()).await;

    let processing_time = start_time.elapsed();

    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert_eq!(batch_result.len(), test_images.len());

    // 性能检查：处理50个图片应该在合理时间内完成
    assert!(
        processing_time.as_secs() < 30,
        "Processing took too long: {processing_time:?}"
    );

    println!(
        "Processed {} images in {:?}",
        test_images.len(),
        processing_time
    );
}

#[tokio::test]
async fn test_concurrent_processing() {
    let temp_dir = TempDir::new().unwrap();

    // 创建多个处理器实例
    let config1 = ImageProcessorConfig::default();
    let config2 = ImageProcessorConfig::default();
    let processor1 = ImageProcessor::new(config1, None);
    let processor2 = ImageProcessor::new(config2, None);

    // 创建测试图片
    let test_images1 = create_test_images(&temp_dir).await;
    let test_images2 = create_test_images(&temp_dir).await;

    // 并发处理
    let (result1, result2): (
        anyhow::Result<Vec<ImageUploadResult>>,
        anyhow::Result<Vec<ImageUploadResult>>,
    ) = tokio::join!(
        processor1.batch_upload_images(test_images1.clone()),
        processor2.batch_upload_images(test_images2.clone())
    );

    assert!(result1.is_ok());
    assert!(result2.is_ok());

    let batch_result1 = result1.unwrap();
    let batch_result2 = result2.unwrap();

    assert_eq!(batch_result1.len(), test_images1.len());
    assert_eq!(batch_result2.len(), test_images2.len());
}

#[tokio::test]
async fn test_markdown_image_replacement() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 创建测试图片
    let image_path = temp_dir.path().join("test.jpg");
    create_test_image(&image_path).await;

    // 创建包含图片的Markdown内容
    let markdown_content = format!(
        "# Test Document\n\n![Test Image]({})\n\nSome text here.",
        image_path.to_string_lossy()
    );

    // 测试替换（由于没有OSS服务，图片路径不会被替换）
    let result = processor.replace_markdown_images(&markdown_content).await;
    assert!(result.is_ok());

    let processed_content = result.unwrap();
    // 由于没有OSS服务，内容应该保持不变
    assert_eq!(processed_content, markdown_content);
}

#[tokio::test]
async fn test_image_validation() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 创建有效的图片文件
    let valid_image = temp_dir.path().join("valid.jpg");
    create_test_image(&valid_image).await;

    // 创建无效的文件（非图片格式）
    let invalid_file = temp_dir.path().join("invalid.txt");
    tokio::fs::write(&invalid_file, "not an image")
        .await
        .unwrap();

    // 测试验证
    let valid_result = processor
        .validate_image_file(valid_image.to_str().unwrap())
        .await;
    assert!(valid_result.is_ok());
    assert!(valid_result.unwrap());

    let invalid_result = processor
        .validate_image_file(invalid_file.to_str().unwrap())
        .await;
    assert!(invalid_result.is_ok());
    assert!(!invalid_result.unwrap());

    // 测试不存在的文件
    let nonexistent_result = processor.validate_image_file("/nonexistent/file.jpg").await;
    assert!(nonexistent_result.is_ok());
    assert!(!nonexistent_result.unwrap());
}

#[tokio::test]
async fn test_extract_image_paths_from_markdown() {
    let markdown_content = r#"
# Test Document

![Image 1](images/test1.jpg)
![Image 2](./local/test2.png)
![External Image](https://example.com/image.jpg)
![Another Image](../parent/test3.gif)

Some text here.
"#;

    let paths = ImageProcessor::extract_image_paths(markdown_content);

    // 应该提取到3个本地图片路径（排除外部URL）
    assert_eq!(paths.len(), 3);
    assert!(paths.contains(&"images/test1.jpg".to_string()));
    assert!(paths.contains(&"./local/test2.png".to_string()));
    assert!(paths.contains(&"../parent/test3.gif".to_string()));
}

#[tokio::test]
async fn test_cache_functionality() {
    let temp_dir = TempDir::new().unwrap();
    let config = ImageProcessorConfig::default();
    let processor = ImageProcessor::new(config, None);

    // 初始缓存应该为空
    let (total, successful) = processor.get_cache_stats().await;
    assert_eq!(total, 0);
    assert_eq!(successful, 0);

    // 尝试上传一些图片（由于没有OSS服务，会失败且不会被缓存）
    let test_images = create_test_images(&temp_dir).await;
    let result = processor.batch_upload_images(test_images).await;

    // 验证上传结果
    assert!(result.is_ok());
    let upload_results = result.unwrap();
    assert!(!upload_results.is_empty());

    // 由于没有OSS服务，所有上传都应该失败
    for upload_result in &upload_results {
        assert!(!upload_result.success);
        assert!(upload_result.error_message.is_some());
    }

    // 检查缓存统计（失败的上传不会被缓存）
    let (total_after, successful_after) = processor.get_cache_stats().await;
    assert_eq!(total_after, 0); // 失败的上传不会被缓存
    assert_eq!(successful_after, 0);

    // 清空缓存（即使为空也应该正常工作）
    processor.clear_cache().await;
    let (total_cleared, successful_cleared) = processor.get_cache_stats().await;
    assert_eq!(total_cleared, 0);
    assert_eq!(successful_cleared, 0);
}

// 辅助函数

async fn create_test_images(temp_dir: &TempDir) -> Vec<String> {
    let mut images = Vec::new();

    for (i, ext) in ["jpg", "png", "gif"].iter().enumerate() {
        let image_path = temp_dir.path().join(format!("test_{i}.{ext}"));
        create_test_image(&image_path).await;
        images.push(image_path.to_string_lossy().to_string());
    }

    images
}

async fn create_test_image(path: &Path) {
    // 创建模拟图片文件（简单的二进制数据）
    let mut content = Vec::new();

    // 根据扩展名添加相应的文件头
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => {
                content.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]); // JPEG header
            }
            "png" => {
                content.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47]); // PNG header
            }
            "gif" => {
                content.extend_from_slice(&[0x47, 0x49, 0x46, 0x38]); // GIF header
            }
            _ => {
                content.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0]); // Default to JPEG
            }
        }
    }

    // 添加一些随机数据
    for i in 0..1024 {
        content.push((i % 256) as u8);
    }

    tokio::fs::write(path, &content).await.unwrap();
}
