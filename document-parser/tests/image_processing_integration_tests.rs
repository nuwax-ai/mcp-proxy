use document_parser::services::image_processor::{
    ImageProcessor, ImageProcessConfig, ImageProcessorConfig, BatchProcessResult
};
use tempfile::TempDir;
use tokio;
use std::path::Path;

#[tokio::test]
async fn test_image_processing_pipeline() {
    let temp_dir = TempDir::new().unwrap();
    let processor = ImageProcessor::with_defaults(
        temp_dir.path().to_path_buf(),
        None,
    );
    
    // 创建测试图片文件
    let test_images = create_test_images(&temp_dir).await;
    
    // 测试批量处理
    let config = ImageProcessConfig::default();
    let result = processor.process_images_batch(&test_images, Some(&config)).await;
    
    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert!(batch_result.successful_results.len() > 0);
    assert_eq!(batch_result.total_processed, test_images.len());
}

#[tokio::test]
async fn test_image_extraction_from_directory() {
    let temp_dir = TempDir::new().unwrap();
    let processor = ImageProcessor::with_defaults(
        temp_dir.path().to_path_buf(),
        None,
    );
    
    // 创建测试目录结构
    let test_dir = temp_dir.path().join("images");
    tokio::fs::create_dir_all(&test_dir).await.unwrap();
    
    // 创建测试图片
    create_test_image(&test_dir.join("test1.jpg")).await;
    create_test_image(&test_dir.join("test2.png")).await;
    
    // 创建子目录
    let sub_dir = test_dir.join("subdir");
    tokio::fs::create_dir_all(&sub_dir).await.unwrap();
    create_test_image(&sub_dir.join("test3.gif")).await;
    
    // 测试提取
    let result = processor.extract_images_from_directory(
        test_dir.to_str().unwrap()
    ).await;
    
    assert!(result.is_ok());
    let extraction_result = result.unwrap();
    assert_eq!(extraction_result.total_files, 3);
    assert!(extraction_result.image_paths.len() == 3);
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let processor = ImageProcessor::with_defaults(
        temp_dir.path().to_path_buf(),
        None,
    );
    
    // 测试不存在的文件
    let non_existent_files = vec![
        "/non/existent/file1.jpg".to_string(),
        "/non/existent/file2.png".to_string(),
    ];
    
    let config = ImageProcessConfig::default();
    let result = processor.process_images_batch(&non_existent_files, Some(&config)).await;
    
    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert_eq!(batch_result.successful_results.len(), 0);
    assert_eq!(batch_result.failed_items.len(), 2);
}

#[tokio::test]
async fn test_performance_with_large_batch() {
    let temp_dir = TempDir::new().unwrap();
    let processor = ImageProcessor::with_defaults(
        temp_dir.path().to_path_buf(),
        None,
    );
    
    // 创建大量测试图片
    let mut test_images = Vec::new();
    for i in 0..50 {
        let image_path = temp_dir.path().join(format!("test_{}.jpg", i));
        create_test_image(&image_path).await;
        test_images.push(image_path.to_string_lossy().to_string());
    }
    
    let start_time = std::time::Instant::now();
    
    let config = ImageProcessConfig::default();
    let result = processor.process_images_batch(&test_images, Some(&config)).await;
    
    let processing_time = start_time.elapsed();
    
    assert!(result.is_ok());
    let batch_result = result.unwrap();
    assert!(batch_result.successful_results.len() > 0);
    
    // 性能检查：处理50个图片应该在合理时间内完成
    assert!(processing_time.as_secs() < 30, "Processing took too long: {:?}", processing_time);
    
    println!("Processed {} images in {:?}", test_images.len(), processing_time);
}

#[tokio::test]
async fn test_concurrent_processing() {
    let temp_dir = TempDir::new().unwrap();
    
    // 创建多个处理器实例
    let processor1 = ImageProcessor::with_defaults(
        temp_dir.path().join("proc1"),
        None,
    );
    let processor2 = ImageProcessor::with_defaults(
        temp_dir.path().join("proc2"),
        None,
    );
    
    // 创建测试图片
    let test_images1 = create_test_images(&temp_dir).await;
    let test_images2 = create_test_images(&temp_dir).await;
    
    let config = ImageProcessConfig::default();
    
    // 并发处理
    let (result1, result2) = tokio::join!(
        processor1.process_images_batch(&test_images1, Some(&config)),
        processor2.process_images_batch(&test_images2, Some(&config))
    );
    
    assert!(result1.is_ok());
    assert!(result2.is_ok());
    
    let batch_result1 = result1.unwrap();
    let batch_result2 = result2.unwrap();
    
    assert!(batch_result1.successful_results.len() > 0);
    assert!(batch_result2.successful_results.len() > 0);
}

// 辅助函数

async fn create_test_images(temp_dir: &TempDir) -> Vec<String> {
    let mut images = Vec::new();
    
    for (i, ext) in ["jpg", "png", "gif"].iter().enumerate() {
        let image_path = temp_dir.path().join(format!("test_{}.{}", i, ext));
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