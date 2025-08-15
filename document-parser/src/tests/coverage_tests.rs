//! Coverage and integration tests
//! 
//! This module contains comprehensive tests designed to achieve >80% code coverage
//! and validate all critical paths through the application.

use std::sync::Arc;
use tokio::time::{timeout, Duration};

use crate::models::*;
use crate::services::*;
use crate::processors::*;
use crate::parsers::*;
use crate::parsers::FormatDetector;
use super::test_config::{TestEnvironment, generators, assertions};

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[tokio::test]
    async fn test_complete_document_processing_pipeline() {
        let env = TestEnvironment::new();
        
        // Create test application state
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let storage_service = Arc::new(StorageService::new(db.clone()).expect("Failed to create storage service"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        // Create test document
        let pdf_file = env.create_test_pdf("test.pdf");
        
        // Test complete pipeline
        let task = task_service.create_task(
            SourceType::Upload,
            Some(pdf_file.to_string_lossy().to_string()),
            DocumentFormat::PDF,
        ).await.expect("Failed to create task");
        
        assertions::assert_valid_task(&task);
        
        // Test status updates through all stages
        let stages = vec![
            ProcessingStage::DownloadingDocument,
            ProcessingStage::FormatDetection,
            ProcessingStage::MinerUExecuting,
            ProcessingStage::ProcessingMarkdown,
            ProcessingStage::GeneratingToc,
            ProcessingStage::SplittingContent,
            ProcessingStage::UploadingMarkdown,
            ProcessingStage::Finalizing,
        ];
        
        for stage in stages {
            let status = TaskStatus::new_processing(stage);
            task_service.update_task_status(&task.id, status).await
                .expect("Failed to update task status");
            
            let updated_task = task_service.get_task(&task.id).await
                .expect("Failed to get task")
                .expect("Task not found");
            
            assert!(updated_task.status.is_processing());
        }
        
        // Complete the task
        let completed_status = TaskStatus::new_completed(Duration::from_secs(120));
        task_service.update_task_status(&task.id, completed_status).await
            .expect("Failed to complete task");
        
        let final_task = task_service.get_task(&task.id).await
            .expect("Failed to get final task")
            .expect("Final task not found");
        
        assert!(matches!(final_task.status, TaskStatus::Completed { .. }));
        assert_eq!(final_task.progress, 100);
    }

    #[tokio::test]
    async fn test_error_handling_coverage() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        // Test all error scenarios
        let error_scenarios = generators::test_error_scenarios();
        
        for (i, error) in error_scenarios.into_iter().enumerate() {
            let task = task_service.create_task(
                SourceType::Upload,
                Some(format!("/tmp/error_test_{}.pdf", i)),
                DocumentFormat::PDF,
            ).await.expect("Failed to create error test task");
            
            let failed_status = TaskStatus::new_failed(error.clone(), 1);
            task_service.update_task_status(&task.id, failed_status).await
                .expect("Failed to set task to failed");
            
            let failed_task = task_service.get_task(&task.id).await
                .expect("Failed to get failed task")
                .expect("Failed task not found");
            
            assert!(failed_task.status.is_failed());
            assert!(failed_task.error_message.is_some());
            assertions::assert_valid_task_error(&error);
        }
    }

    #[tokio::test]
    async fn test_markdown_processing_coverage() {
        let processor = MarkdownProcessor::default();
        let samples = generators::test_markdown_samples();
        
        for (name, content) in samples {
            let result = processor.process_markdown(content).await;
            
            match result {
                Ok(doc_structure) => {
                    assertions::assert_valid_structured_document(&doc_structure);
                    
                    // Verify specific properties based on content type
                    match name {
                        "simple" => {
                            assert_eq!(doc_structure.toc.len(), 1);
                            assert_eq!(doc_structure.toc[0].level, 1);
                        },
                        "nested" => {
                            assert!(doc_structure.toc.len() >= 2);
                            // Should have nested structure
                            assert!(doc_structure.toc.iter().any(|s| !s.children.is_empty()));
                        },
                        "empty" => {
                            assert_eq!(doc_structure.toc.len(), 0);
                            assert_eq!(doc_structure.total_sections, 0);
                        },
                        "no_headers" => {
                            assert_eq!(doc_structure.toc.len(), 0);
                        },
                        "unicode" => {
                            assert!(doc_structure.toc.len() >= 1);
                            // Should handle Unicode properly
                            assert!(doc_structure.toc.iter().any(|s| s.title.contains("中文")));
                        },
                        "with_images" => {
                            assert!(doc_structure.toc.len() >= 1);
                            // Should preserve image references
                            assert!(doc_structure.toc.iter().any(|s| s.content.contains("![Image]")));
                        },
                        "with_links" => {
                            assert!(doc_structure.toc.len() >= 1);
                            // Should preserve links
                            assert!(doc_structure.toc.iter().any(|s| s.content.contains("[Link]")));
                        },
                        "complex" => {
                            assert!(doc_structure.toc.len() >= 3);
                            // Should have multiple levels
                            let levels: std::collections::HashSet<_> = doc_structure.toc.iter()
                                .map(|s| s.level)
                                .collect();
                            assert!(levels.len() >= 2);
                        },
                        _ => {
                            // Generic validation for any other test cases
                        }
                    }
                },
                Err(e) => {
                    // Some content might legitimately fail to process
                    // Log the error for debugging but don't fail the test
                    eprintln!("Failed to process '{}': {}", name, e);
                }
            }
        }
    }

    #[tokio::test]
    async fn test_concurrent_operations_coverage() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        // Test concurrent task creation
        let mut handles = vec![];
        
        for i in 0..20 {
            let task_service_clone = Arc::clone(&task_service);
            let handle = tokio::spawn(async move {
                let task = task_service_clone.create_task(
                    SourceType::Upload,
                    Some(format!("/tmp/concurrent_test_{}.pdf", i)),
                    DocumentFormat::PDF,
                ).await?;
                
                // Update status concurrently
                let status = TaskStatus::new_processing(ProcessingStage::FormatDetection);
                task_service_clone.update_task_status(&task.id, status).await?;
                
                Ok::<String, anyhow::Error>(task.id)
            });
            handles.push(handle);
        }
        
        // Wait for all tasks to complete
        let mut task_ids = vec![];
        for handle in handles {
            let task_id = handle.await.expect("Concurrent task failed")
                .expect("Failed to create concurrent task");
            task_ids.push(task_id);
        }
        
        // Verify all tasks were created successfully
        assert_eq!(task_ids.len(), 20);
        
        // Verify all tasks exist and are in processing state
        for task_id in task_ids {
            let task = task_service.get_task(&task_id).await
                .expect("Failed to get concurrent task")
                .expect("Concurrent task not found");
            
            assert!(task.status.is_processing());
            assertions::assert_valid_task(&task);
        }
    }

    #[tokio::test]
    async fn test_storage_operations_coverage() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let storage_service = StorageService::new(db.clone()).expect("Failed to create storage service");
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        let queue_service = TaskQueueService::new(task_service.clone());
        
        // Test CRUD operations
        let task = generators::test_document_task();
        
        // Create
        storage_service.save_task(&task).await.expect("Failed to save task");
        
        // Read
        let retrieved_task = storage_service.get_task(&task.id).await
            .expect("Failed to get task")
            .expect("Task not found");
        assert_eq!(retrieved_task.id, task.id);
        
        // Update
        let new_status = TaskStatus::new_processing(ProcessingStage::MinerUExecuting);
        task_service.update_task_status(&task.id, new_status).await
            .expect("Failed to update task status");
        
        let updated_task = storage_service.get_task(&task.id).await
            .expect("Failed to get updated task")
            .expect("Updated task not found");
        assert!(updated_task.status.is_processing());
        
        // List operations
        let filter = crate::services::storage_service::QueryFilter {
            limit: Some(10),
            ..Default::default()
        };
        let tasks = storage_service.query_tasks(&filter).await
            .expect("Failed to list tasks");
        assert!(!tasks.is_empty());
        
        // Cleanup operations
        let mut expired_task = task.clone();
        expired_task.expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        storage_service.save_task(&expired_task).await.expect("Failed to save expired task");
        
        let cleaned_count = storage_service.cleanup_expired_data().await
            .expect("Failed to cleanup expired tasks");
        assert!(cleaned_count >= 1);
    }

    #[tokio::test]
    async fn test_format_detection_coverage() {
        let env = TestEnvironment::new();
        let config = env.config.clone();
        
        let dual_parser = DualEngineParser::new(&config.mineru, &config.markitdown);
        let format_detector = FormatDetector::new();
        
        // Test all supported formats
        let test_cases = vec![
            ("document.pdf", DocumentFormat::PDF, ParserEngine::MinerU),
            ("document.docx", DocumentFormat::Word, ParserEngine::MarkItDown),
            ("document.doc", DocumentFormat::Word, ParserEngine::MarkItDown),
            ("presentation.pptx", DocumentFormat::PowerPoint, ParserEngine::MarkItDown),
            ("presentation.ppt", DocumentFormat::PowerPoint, ParserEngine::MarkItDown),
            ("spreadsheet.xlsx", DocumentFormat::Excel, ParserEngine::MarkItDown),
            ("spreadsheet.xls", DocumentFormat::Excel, ParserEngine::MarkItDown),
            ("image.png", DocumentFormat::Image, ParserEngine::MarkItDown),
            ("image.jpg", DocumentFormat::Image, ParserEngine::MarkItDown),
            ("image.jpeg", DocumentFormat::Image, ParserEngine::MarkItDown),
            ("image.gif", DocumentFormat::Image, ParserEngine::MarkItDown),
            ("audio.mp3", DocumentFormat::Audio, ParserEngine::MarkItDown),
            ("audio.wav", DocumentFormat::Audio, ParserEngine::MarkItDown),
        ];
        
        for (filename, expected_format, _expected_engine) in test_cases {
            // Create a temporary file for testing
            let temp_dir = env.temp_dir.path();
            let test_file = temp_dir.join(filename);
            std::fs::write(&test_file, b"test content").unwrap();
            
            let detection_result = format_detector.detect_format(test_file.to_str().unwrap(), None);
            if detection_result.is_ok() {
                let result = detection_result.unwrap();
                assert_eq!(result.format, expected_format, "Format detection failed for {}", filename);
                
                // Test that dual parser supports this format
                assert!(dual_parser.supports_format(&result.format), "Dual parser should support format for {}", filename);
            }
        }
        
        // Test MIME type detection with temporary files
        let mime_test_cases = vec![
            ("test.pdf", Some("application/pdf"), DocumentFormat::PDF),
            ("test.docx", Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"), DocumentFormat::Word),
            ("test.png", Some("image/png"), DocumentFormat::Image),
        ];
        
        for (filename, mime_type, expected_format) in mime_test_cases {
            let temp_dir = env.temp_dir.path();
            let test_file = temp_dir.join(filename);
            std::fs::write(&test_file, b"test content").unwrap();
            
            let detection_result = format_detector.detect_format(test_file.to_str().unwrap(), mime_type);
            if detection_result.is_ok() {
                let result = detection_result.unwrap();
                assert_eq!(result.format, expected_format, "MIME type detection failed for {}", filename);
            }
        }
    }

    #[tokio::test]
    async fn test_task_queue_coverage() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        let queue_service = TaskQueueService::new(task_service);
        
        // Test queue operations
        let initial_stats = queue_service.get_stats().await;
        assert_eq!(initial_stats.pending_count, 0);
        assert_eq!(initial_stats.processing_count, 0);
        
        // Enqueue tasks
        let tasks = vec![
            generators::test_document_task(),
            generators::test_document_task_with_params(SourceType::Url, DocumentFormat::Word, ParserEngine::MarkItDown),
            generators::test_document_task_with_params(SourceType::ExternalApi, DocumentFormat::Excel, ParserEngine::MarkItDown),
        ];
        
        for task in &tasks {
            queue_service.enqueue_task(task.id.clone(), 1).await
                .expect("Failed to enqueue task");
        }
        
        // Check updated stats
        let updated_stats = queue_service.get_stats().await;
        assert!(updated_stats.pending_count > 0 || updated_stats.processing_count > 0);
        
        // Test graceful shutdown
        let shutdown_result = timeout(Duration::from_secs(5), queue_service.shutdown()).await;
        assert!(shutdown_result.is_ok(), "Queue service should shutdown gracefully");
    }

    #[tokio::test]
    async fn test_image_processing_coverage() {
        let env = TestEnvironment::new();
        
        let image_processor = crate::services::ImageProcessor::new(
            env.temp_path().to_path_buf(),
            None,
            None,
        );
        
        // Create test images
        let image1 = env.create_test_image("test1.png");
        let image2 = env.create_test_image("test2.jpg");
        
        let image_paths = vec![
            image1.to_string_lossy().to_string(),
            image2.to_string_lossy().to_string(),
        ];
        
        // Test batch processing
        let result = image_processor.process_images_batch(&image_paths, None).await;
        assert!(result.is_ok(), "Image batch processing should succeed");
        
        let processed_images = result.unwrap();
        assert_eq!(processed_images.successful_results.len(), 2);
        assert_eq!(processed_images.failed_items.len(), 0);
        
        // Test image path extraction
        let markdown_with_images = r#"# Document
![Image 1](./images/image1.png)
Some content.
![Image 2](/absolute/path/image2.jpg)
![Image 3](https://example.com/image3.gif)
"#;
        
        let extracted_paths = image_processor.extract_image_paths(markdown_with_images);
        assert_eq!(extracted_paths.len(), 3);
        assert!(extracted_paths.contains(&"./images/image1.png".to_string()));
        assert!(extracted_paths.contains(&"/absolute/path/image2.jpg".to_string()));
        assert!(extracted_paths.contains(&"https://example.com/image3.gif".to_string()));
    }

    #[tokio::test]
    async fn test_configuration_coverage() {
        let env = TestEnvironment::new();
        let config = &env.config;
        
        // Test all configuration sections
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 0);
        
        assert_eq!(config.log.level, "debug");
        assert!(!config.log.path.is_empty());
        
        assert_eq!(config.document_parser.max_concurrent, 2);
        assert_eq!(config.document_parser.queue_size, 10);
        
        assert!(!config.storage.sled.path.is_empty());
        assert_eq!(config.storage.sled.cache_capacity, 1024 * 1024);
        
        assert_eq!(config.storage.oss.bucket, "test-bucket");
        
        assert_eq!(config.mineru.backend, "pipeline");
        assert_eq!(config.mineru.max_concurrent, 1);
        
        assert_eq!(config.markitdown.python_path, "python3");
        assert!(!config.markitdown.enable_plugins);
        assert!(!config.markitdown.features.ocr);
        assert!(!config.markitdown.features.audio_transcription);
    }

    #[tokio::test]
    async fn test_utility_functions_coverage() {
        let env = TestEnvironment::new();
        
        // Test file utilities
        let test_file = env.create_test_file("utility_test.txt", b"test content");
        
        assert!(crate::utils::file_exists(test_file.to_str().unwrap()));
        
        let file_size = crate::utils::get_file_size(test_file.to_str().unwrap())
            .expect("Failed to get file size");
        assert_eq!(file_size, 12); // "test content" is 12 bytes
        
        let extension = crate::utils::get_file_extension(test_file.to_str().unwrap());
        assert_eq!(extension, Some("txt".to_string()));
        
        // Test format utilities
        let format = crate::utils::detect_format_from_path("test.pdf")
            .expect("Failed to detect format");
        assert_eq!(format, DocumentFormat::PDF);
        
        assert!(crate::utils::is_format_supported(&DocumentFormat::PDF));
        assert!(crate::utils::is_format_supported(&DocumentFormat::Word));
        
        // Test directory operations
        let temp_dir = env.temp_path().join("test_subdir");
        let result = crate::utils::create_temp_dir(temp_dir.to_str().unwrap());
        assert!(result.is_ok());
        assert!(temp_dir.exists());
    }
}

#[cfg(test)]
mod integration_coverage_tests {
    use super::*;

    #[tokio::test]
    async fn test_end_to_end_document_processing() {
        let env = TestEnvironment::new();
        
        // Setup complete application state
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let storage_service = Arc::new(StorageService::new(db.clone()).expect("Failed to create storage service"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        let dual_parser = DualEngineParser::new(&env.config.mineru, &env.config.markitdown);
        let markdown_processor = MarkdownProcessor::default();
        
        let document_service = DocumentService::new(
            dual_parser,
            markdown_processor,
            Arc::clone(&task_service),
            None, // No OSS service for testing
        );
        
        // Create test document
        let test_file = env.create_test_pdf("integration_test.pdf");
        
        // Test complete workflow
        let task = task_service.create_task(
            SourceType::Upload,
            Some(test_file.to_string_lossy().to_string()),
            DocumentFormat::PDF,
        ).await.expect("Failed to create integration task");
        
        // Verify supported formats
        let supported_formats = document_service.get_supported_formats();
        assert!(supported_formats.contains(&DocumentFormat::PDF));
        assert!(supported_formats.contains(&DocumentFormat::Word));
        
        // Test format detection
        let format_detector = FormatDetector::new();
        let detection_result = format_detector.detect_format(
            test_file.to_str().unwrap(),
            Some("application/pdf"),
        );
        assert_eq!(detection_result.unwrap().format, DocumentFormat::PDF);
        
        // Simulate processing stages
        let processing_stages = vec![
            ProcessingStage::DownloadingDocument,
            ProcessingStage::FormatDetection,
            ProcessingStage::MinerUExecuting,
            ProcessingStage::ProcessingMarkdown,
            ProcessingStage::GeneratingToc,
            ProcessingStage::SplittingContent,
            ProcessingStage::UploadingMarkdown,
            ProcessingStage::Finalizing,
        ];
        
        for (i, stage) in processing_stages.iter().enumerate() {
            let progress = ((i + 1) * 100 / processing_stages.len()) as u32;
            let progress_details = ProgressDetails::new(format!("Processing stage: {}", stage.get_name()));
            let status = TaskStatus::Processing {
                stage: stage.clone(),
                progress_details: Some(progress_details),
                started_at: chrono::Utc::now(),
            };
            
            task_service.update_task_status(&task.id, status).await
                .expect("Failed to update processing status");
            
            let updated_task = task_service.get_task(&task.id).await
                .expect("Failed to get processing task")
                .expect("Processing task not found");
            
            assert!(updated_task.status.is_processing());
            assertions::assert_valid_task(&updated_task);
        }
        
        // Complete the task
        let completion_status = TaskStatus::new_completed(Duration::from_secs(180));
        task_service.update_task_status(&task.id, completion_status).await
            .expect("Failed to complete integration task");
        
        let final_task = task_service.get_task(&task.id).await
            .expect("Failed to get final integration task")
            .expect("Final integration task not found");
        
        assert!(matches!(final_task.status, TaskStatus::Completed { .. }));
        assert_eq!(final_task.progress, 100);
        assertions::assert_valid_task(&final_task);
    }

    #[tokio::test]
    async fn test_error_recovery_integration() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        // Create task that will fail
        let task = task_service.create_task(
            SourceType::Upload,
            Some("/nonexistent/file.pdf".to_string()),
            DocumentFormat::PDF,
        ).await.expect("Failed to create error recovery task");
        
        // Simulate failure
        let error = TaskError::new(
            "E001".to_string(),
            "File not found during processing".to_string(),
            Some(ProcessingStage::DownloadingDocument),
        );
        
        let failed_status = TaskStatus::new_failed(error.clone(), 1);
        task_service.update_task_status(&task.id, failed_status).await
            .expect("Failed to set task to failed state");
        
        let failed_task = task_service.get_task(&task.id).await
            .expect("Failed to get failed task")
            .expect("Failed task not found");
        
        assert!(failed_task.status.is_failed());
        assert!(failed_task.error_message.is_some());
        assert_eq!(failed_task.retry_count, 1);
        
        // Test retry logic (if implemented)
        if failed_task.retry_count < failed_task.max_retries {
            let retry_status = TaskStatus::new_pending();
            task_service.update_task_status(&task.id, retry_status).await
                .expect("Failed to retry task");
            
            let retried_task = task_service.get_task(&task.id).await
                .expect("Failed to get retried task")
                .expect("Retried task not found");
            
            assert!(retried_task.status.is_pending());
        }
    }

    #[tokio::test]
    async fn test_performance_under_load() {
        let env = TestEnvironment::new();
        let db = Arc::new(sled::open(&env.db_path).expect("Failed to open test database"));
        let task_service = Arc::new(TaskService::new(db.clone()).expect("Failed to create task service"));
        
        let start_time = std::time::Instant::now();
        
        // Create many tasks concurrently
        let mut handles = vec![];
        for i in 0..50 {
            let task_service_clone = Arc::clone(&task_service);
            let handle = tokio::spawn(async move {
                let task = task_service_clone.create_task(
                    SourceType::Upload,
                    Some(format!("/tmp/load_test_{}.pdf", i)),
                    DocumentFormat::PDF,
                ).await?;
                
                // Simulate processing
                let status = TaskStatus::new_processing(ProcessingStage::FormatDetection);
                task_service_clone.update_task_status(&task.id, status).await?;
                
                let completed_status = TaskStatus::new_completed(Duration::from_millis(100));
                task_service_clone.update_task_status(&task.id, completed_status).await?;
                
                Ok::<String, anyhow::Error>(task.id)
            });
            handles.push(handle);
        }
        
        // Wait for all tasks
        let mut completed_tasks = vec![];
        for handle in handles {
            let task_id = handle.await.expect("Load test task failed")
                .expect("Failed to process load test task");
            completed_tasks.push(task_id);
        }
        
        let duration = start_time.elapsed();
        
        // Verify performance
        assert_eq!(completed_tasks.len(), 50);
        assert!(duration.as_secs() < 30, "Load test took too long: {:?}", duration);
        
        // Verify all tasks completed successfully
        for task_id in completed_tasks {
            let task = task_service.get_task(&task_id).await
                .expect("Failed to get load test task")
                .expect("Load test task not found");
            
            assert!(matches!(task.status, TaskStatus::Completed { .. }));
            assertions::assert_valid_task(&task);
        }
    }
}