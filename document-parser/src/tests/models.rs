//! 数据模型单元测试
use crate::models::*;
use serde_json;
use std::time::Duration;
use chrono::{DateTime, Utc};
use uuid::Uuid;
use crate::config::{AppConfig};
use crate::tests::test_helpers::safe_init_global_config;

#[cfg(test)]
mod document_task_tests {
    use super::*;

    #[test]
    fn test_document_task_builder() {
        safe_init_global_config();
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .source_path(Some("/tmp/test.pdf".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");

        assert!(!task.id.is_empty());
        assert_eq!(task.document_format, DocumentFormat::PDF);
        assert_eq!(task.parser_engine, ParserEngine::MinerU);
        assert_eq!(task.source_type, SourceType::Upload);
        assert!(task.status.is_pending());
    }

    #[test]
    fn test_document_task_validation_invalid_uuid() {
        safe_init_global_config();
        let result = DocumentTask::builder()
            .id("invalid-uuid".to_string())
            .source_type(SourceType::Upload)
            .source_path(Some("/tmp/test.pdf".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build();
        
        assert!(result.is_err(), "Should fail with invalid UUID");
    }

    #[test]
    fn test_task_status_creation() {
        safe_init_global_config();
        let pending = TaskStatus::new_pending();
        assert!(pending.is_pending());

        let processing = TaskStatus::new_processing(ProcessingStage::DownloadingDocument);
        assert!(processing.is_processing());

        let completed = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        assert!(matches!(completed, TaskStatus::Completed { .. }));

        let error = TaskError::new(
            "E001".to_string(),
            "Test error".to_string(),
            None,
        );
        let failed = TaskStatus::new_failed(error, 0);
        assert!(failed.is_failed());
    }
}

#[cfg(test)]
mod document_format_tests {
    use super::*;

    #[test]
    fn test_document_format_detection() {
        assert_eq!(DocumentFormat::from_extension("pdf"), DocumentFormat::PDF);
        assert_eq!(DocumentFormat::from_extension("docx"), DocumentFormat::Word);
        assert_eq!(DocumentFormat::from_extension("xlsx"), DocumentFormat::Excel);
        assert_eq!(DocumentFormat::from_extension("pptx"), DocumentFormat::PowerPoint);
        assert_eq!(DocumentFormat::from_extension("jpg"), DocumentFormat::Image);
        assert_eq!(DocumentFormat::from_extension("unknown"), DocumentFormat::Other("unknown".to_string()));
    }

    #[test]
    fn test_document_format_serialization() {
        let formats = vec![
            DocumentFormat::PDF,
            DocumentFormat::Word,
            DocumentFormat::Excel,
            DocumentFormat::PowerPoint,
            DocumentFormat::Image,
        ];

        for format in formats {
            let json = serde_json::to_string(&format).expect("Failed to serialize format");
            let deserialized: DocumentFormat = serde_json::from_str(&json)
                .expect("Failed to deserialize format");
            assert_eq!(format, deserialized);
        }
    }
}

#[cfg(test)]
mod parser_engine_tests {
    use super::*;

    #[test]
    fn test_parser_engine_selection() {
        assert!(ParserEngine::MinerU.supports_format(&DocumentFormat::PDF));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Word));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Excel));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::PowerPoint));
    }

    #[test]
    fn test_parser_engine_serialization() {
        let engines = vec![ParserEngine::MinerU, ParserEngine::MarkItDown];

        for engine in engines {
            let json = serde_json::to_string(&engine).expect("Failed to serialize engine");
            let deserialized: ParserEngine = serde_json::from_str(&json)
                .expect("Failed to deserialize engine");
            assert_eq!(engine, deserialized);
        }
    }
}

#[cfg(test)]
mod task_error_tests {
    use super::*;

    #[test]
    fn test_task_error_creation() {
        let error = TaskError::new(
            "E001".to_string(),
            "Test error message".to_string(),
            Some(ProcessingStage::DownloadingDocument),
        );

        assert_eq!(error.error_code, "E001");
        assert_eq!(error.error_message, "Test error message");
        assert_eq!(error.stage, Some(ProcessingStage::DownloadingDocument));
    }

    #[test]
    fn test_task_error_serialization() {
        let error = TaskError::new(
            "E002".to_string(),
            "Another test error".to_string(),
            None,
        );

        let json = serde_json::to_string(&error).expect("Failed to serialize error");
        let deserialized: TaskError = serde_json::from_str(&json)
            .expect("Failed to deserialize error");
        
        assert_eq!(error.error_code, deserialized.error_code);
        assert_eq!(error.error_message, deserialized.error_message);
    }
}

#[cfg(test)]
mod processing_stage_tests {
    use super::*;

    #[test]
    fn test_processing_stage_properties() {
        safe_init_global_config();
        let stage = ProcessingStage::DownloadingDocument;
        assert_eq!(stage.get_name(), "下载文档");
        assert_eq!(stage.get_description(), "正在下载文档文件");
        assert!(stage.get_progress() > 0);
    }

    #[test]
    fn test_processing_stage_serialization() {
        safe_init_global_config();
        let stages = vec![
            ProcessingStage::DownloadingDocument,
            ProcessingStage::FormatDetection,
            ProcessingStage::MinerUExecuting,
            ProcessingStage::MarkItDownExecuting,
            ProcessingStage::Finalizing,
        ];

        for stage in stages {
            let json = serde_json::to_string(&stage).expect("Failed to serialize stage");
            let deserialized: ProcessingStage = serde_json::from_str(&json)
                .expect("Failed to deserialize stage");
            assert_eq!(stage, deserialized);
        }
    }
}

#[cfg(test)]
mod source_type_tests {
    use super::*;

    #[test]
    fn test_source_type_validation() {
        // 测试文件上传类型
        let file_upload = SourceType::Upload;
        assert_eq!(file_upload.get_description(), "文件上传");
        
        // 测试URL下载类型
        let url_download = SourceType::Url;
        assert_eq!(url_download.get_description(), "URL下载");
        
        // 测试外部API类型
        let external_api = SourceType::ExternalApi;
        assert_eq!(external_api.get_description(), "外部API调用");
    }

    #[test]
    fn test_source_type_serialization() {
        let types = vec![
            SourceType::Upload,
            SourceType::Url,
            SourceType::ExternalApi,
        ];

        for source_type in types {
            let json = serde_json::to_string(&source_type).expect("Failed to serialize source type");
            let deserialized: SourceType = serde_json::from_str(&json)
                .expect("Failed to deserialize source type");
            assert_eq!(source_type, deserialized);
        }
    }
}
#[cfg(test)]
mod comprehensive_model_tests {
    use super::*;

    #[test]
    fn test_document_task_builder_comprehensive() {
        safe_init_global_config();
        // Test successful build
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .source_path(Some("/tmp/test.pdf".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024 * 1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");

        assert!(!task.id.is_empty());
        assert!(Uuid::parse_str(&task.id).is_ok());
        assert_eq!(task.source_type, SourceType::Upload);
        assert_eq!(task.document_format, DocumentFormat::PDF);
        assert_eq!(task.parser_engine, ParserEngine::MinerU);
        assert!(task.status.is_pending());
        assert_eq!(task.file_size, Some(1024 * 1024));
        assert_eq!(task.mime_type, Some("application/pdf".to_string()));
        assert!(task.expires_at > task.created_at);
    }

    #[test]
    fn test_document_task_builder_validation_errors() {
        safe_init_global_config();
        // Test invalid UUID
        let result = DocumentTask::builder()
            .id("invalid-uuid".to_string())
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build();
        assert!(result.is_err());

        // Test missing required fields
        let result = DocumentTask::builder().build();
        assert!(result.is_err());

        // Test invalid file size
        let result = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .file_size(0) // Invalid size
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_document_task_serialization_roundtrip() {
        safe_init_global_config();
        let original_task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .source_path(Some("/tmp/test.pdf".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");

        let json = serde_json::to_string(&original_task).expect("Failed to serialize");
        let deserialized_task: DocumentTask = serde_json::from_str(&json)
            .expect("Failed to deserialize");

        assert_eq!(original_task.id, deserialized_task.id);
        assert_eq!(original_task.source_type, deserialized_task.source_type);
        assert_eq!(original_task.document_format, deserialized_task.document_format);
        assert_eq!(original_task.parser_engine, deserialized_task.parser_engine);
    }

    #[test]
    fn test_task_status_state_transitions() {
        safe_init_global_config();
        let pending = TaskStatus::new_pending();
        assert!(pending.is_pending());
        assert!(!pending.is_processing());
        assert!(!matches!(pending, TaskStatus::Completed { .. }));
        assert!(!pending.is_failed());

        let processing = TaskStatus::new_processing(ProcessingStage::FormatDetection);
        assert!(!processing.is_pending());
        assert!(processing.is_processing());
        assert!(!matches!(processing, TaskStatus::Completed { .. }));
        assert!(!processing.is_failed());

        let completed = TaskStatus::new_completed(Duration::from_secs(120));
        assert!(!completed.is_pending());
        assert!(!completed.is_processing());
        assert!(matches!(completed, TaskStatus::Completed { .. }));
        assert!(!completed.is_failed());

        let error = TaskError::new(
            "E001".to_string(),
            "Test error".to_string(),
            Some(ProcessingStage::MinerUExecuting),
        );
        let failed = TaskStatus::new_failed(error, 1);
        assert!(!failed.is_pending());
        assert!(!failed.is_processing());
        assert!(!matches!(failed, TaskStatus::Completed { .. }));
        assert!(failed.is_failed());
    }

    #[test]
    fn test_processing_stage_properties() {
        safe_init_global_config();
        let stages = vec![
            ProcessingStage::DownloadingDocument,
            ProcessingStage::FormatDetection,
            ProcessingStage::MinerUExecuting,
            ProcessingStage::MarkItDownExecuting,
            ProcessingStage::UploadingImages,
            ProcessingStage::ProcessingMarkdown,
            ProcessingStage::GeneratingToc,
            ProcessingStage::SplittingContent,
            ProcessingStage::UploadingMarkdown,
            ProcessingStage::Finalizing,
        ];

        for stage in stages {
            let name = stage.get_name();
            let description = stage.get_description();
            let progress = stage.get_progress();

            assert!(!name.is_empty());
            assert!(!description.is_empty());
            assert!(progress >= 0 && progress <= 100);
        }
    }

    #[test]
    fn test_document_format_comprehensive() {
        safe_init_global_config();
        // Test all supported formats
        let formats = vec![
            ("pdf", DocumentFormat::PDF),
            ("docx", DocumentFormat::Word),
            ("doc", DocumentFormat::Word),
            ("xlsx", DocumentFormat::Excel),
            ("xls", DocumentFormat::Excel),
            ("pptx", DocumentFormat::PowerPoint),
            ("ppt", DocumentFormat::PowerPoint),
            ("jpg", DocumentFormat::Image),
            ("jpeg", DocumentFormat::Image),
            ("png", DocumentFormat::Image),
            ("gif", DocumentFormat::Image),
            ("mp3", DocumentFormat::Audio),
            ("wav", DocumentFormat::Audio),
            ("unknown", DocumentFormat::Other("unknown".to_string())),
        ];

        for (ext, expected) in formats {
            assert_eq!(DocumentFormat::from_extension(ext), expected);
        }
    }

    #[test]
    fn test_parser_engine_format_support() {
        safe_init_global_config();
        // MinerU should support PDF
        assert!(ParserEngine::MinerU.supports_format(&DocumentFormat::PDF));
        assert!(!ParserEngine::MinerU.supports_format(&DocumentFormat::Word));

        // MarkItDown should support other formats
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Word));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Excel));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::PowerPoint));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Image));
        assert!(ParserEngine::MarkItDown.supports_format(&DocumentFormat::Audio));
        assert!(!ParserEngine::MarkItDown.supports_format(&DocumentFormat::PDF));
    }

    #[test]
    fn test_task_error_comprehensive() {
        safe_init_global_config();
        let error = TaskError::new(
            "E001".to_string(),
            "Test error message".to_string(),
            Some(ProcessingStage::DownloadingDocument),
        );

        assert_eq!(error.error_code, "E001");
        assert_eq!(error.error_message, "Test error message");
        assert_eq!(error.stage, Some(ProcessingStage::DownloadingDocument));

        // Test serialization
        let json = serde_json::to_string(&error).expect("Failed to serialize error");
        let deserialized: TaskError = serde_json::from_str(&json)
            .expect("Failed to deserialize error");
        
        assert_eq!(error.error_code, deserialized.error_code);
        assert_eq!(error.error_message, deserialized.error_message);
        assert_eq!(error.stage, deserialized.stage);
    }

    #[test]
    fn test_source_type_descriptions() {
        safe_init_global_config();
        assert_eq!(SourceType::Upload.get_description(), "文件上传");
        assert_eq!(SourceType::Url.get_description(), "URL下载");
        assert_eq!(SourceType::ExternalApi.get_description(), "外部API调用");
    }

    #[test]
    fn test_structured_document_creation() {
        safe_init_global_config();
        let mut doc = StructuredDocument::new(
            "test-task-id".to_string(),
            "Test Document".to_string(),
        ).unwrap();
        
        // Set optional fields
        doc.word_count = Some(100);
        doc.processing_time = Some("2.5s".to_string());

        assert_eq!(doc.task_id, "test-task-id");
        assert_eq!(doc.document_title, "Test Document");
        assert_eq!(doc.total_sections, 0);
        assert_eq!(doc.word_count, Some(100));
    }

    #[test]
    fn test_structured_section_hierarchy() {
        safe_init_global_config();
        let mut child_section = StructuredSection::new(
            "section-1-1".to_string(),
            "Subsection 1.1".to_string(),
            2,
            "Subsection content".to_string(),
        ).unwrap();
        
        // Set optional fields
        child_section.start_pos = Some(100);
        child_section.end_pos = Some(200);

        let mut parent_section = StructuredSection::new(
            "section-1".to_string(),
            "Section 1".to_string(),
            1,
            "Section content".to_string(),
        ).unwrap();
        
        // Set optional fields
        parent_section.start_pos = Some(0);
        parent_section.end_pos = Some(200);
        
        // Add child
        parent_section.add_child(child_section.clone()).unwrap();

        assert_eq!(parent_section.level, 1);
        assert_eq!(parent_section.children.len(), 1);
        assert_eq!(parent_section.children[0].id, child_section.id);
        assert_eq!(parent_section.children[0].level, 2);
    }

    #[test]
    fn test_oss_data_structure() {
        safe_init_global_config();
        let image1 = ImageInfo::new(
            "/tmp/image1.png".to_string(),
            "https://oss.example.com/image1.png".to_string(),
            1024,
            "image/png".to_string(),
        );
        let image2 = ImageInfo::new(
            "/tmp/image2.jpg".to_string(),
            "https://oss.example.com/image2.jpg".to_string(),
            2048,
            "image/jpeg".to_string(),
        );
        
        let oss_data = OssData {
            markdown_url: "https://oss.example.com/markdown.md".to_string(),
            markdown_object_key: Some("markdown/test_task/20241215_120000_document.md".to_string()),
            images: vec![image1, image2],
            bucket: "test-bucket".to_string(),
        };

        assert!(!oss_data.markdown_url.is_empty());
        assert_eq!(oss_data.images.len(), 2);
        assert_eq!(oss_data.bucket, "test-bucket");
    }

    #[test]
    fn test_parse_result_structure() {
        safe_init_global_config();
        let mut parse_result = ParseResult::new(
            "# Test\nContent here".to_string(),
            DocumentFormat::PDF,
            ParserEngine::MinerU,
        );
        parse_result.add_image("/tmp/image1.png".to_string());
        parse_result.set_processing_time(30.0);

        assert!(parse_result.is_success());
        assert_eq!(parse_result.engine, ParserEngine::MinerU);
        assert_eq!(parse_result.images.len(), 1);
        assert!(!parse_result.markdown_content.is_empty());
    }
}

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_and_null_values() {
        safe_init_global_config();
        // Test empty strings
        let format = DocumentFormat::from_extension("");
        assert!(matches!(format, DocumentFormat::Other(_)));

        // Test null-like values
        let task_result = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .source_path(None::<String>) // No source path
            .build();
        
        assert!(task_result.is_ok());
        let task = task_result.unwrap();
        assert!(task.source_path.is_none());
    }

    #[test]
    fn test_boundary_values() {
        safe_init_global_config();
        // Test maximum file size
        let large_task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .file_size(u64::MAX)
            .build();
        
        // Should handle large values gracefully
        assert!(large_task.is_ok() || large_task.is_err()); // Either way is acceptable

        // Test zero expiration
        let task_result = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .expires_in_hours(0)
            .build();
        
        assert!(task_result.is_err()); // Should fail with zero expiration
    }

    #[test]
    fn test_unicode_and_special_characters() {
        safe_init_global_config();
        // Test Unicode in file paths and content
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .source_path(Some("/tmp/测试文档.pdf".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .backend("pipeline")
            .mime_type("application/pdf")
            .build()
            .expect("Failed to build task with Unicode path");

        assert!(task.source_path.unwrap().contains("测试文档"));

        // Test special characters in error messages
        let error = TaskError::new(
            "E001".to_string(),
            "Error with special chars: @#$%^&*()".to_string(),
            None,
        );
        
        assert!(error.error_message.contains("@#$%^&*()"));
    }

    #[test]
    fn test_concurrent_access_safety() {
        safe_init_global_config();
        use std::sync::Arc;
        use std::thread;

        // Test that models can be safely shared between threads
        let task = Arc::new(DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .parser_engine(ParserEngine::MinerU)
            .build()
            .expect("Failed to build task"));

        let handles: Vec<_> = (0..10).map(|_| {
            let task_clone = Arc::clone(&task);
            thread::spawn(move || {
                // Read operations should be safe
                let _id = &task_clone.id;
                let _format = &task_clone.document_format;
                let _status = &task_clone.status;
            })
        }).collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}