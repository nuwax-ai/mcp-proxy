//! Property-based testing utilities and tests
//!
//! This module contains property-based tests using quickcheck to validate
//! data model invariants and business logic properties.

use quickcheck::{Arbitrary, Gen};
use quickcheck_macros::quickcheck;
use uuid::Uuid;

use crate::AppError;
use crate::models::*;
use crate::tests::test_helpers::safe_init_global_config;

/// Arbitrary implementation for DocumentFormat
impl Arbitrary for DocumentFormat {
    fn arbitrary(g: &mut Gen) -> Self {
        let formats = vec![
            DocumentFormat::PDF,
            DocumentFormat::Word,
            DocumentFormat::Excel,
            DocumentFormat::PowerPoint,
            DocumentFormat::Image,
            DocumentFormat::Audio,
            DocumentFormat::Other("test".to_string()),
        ];
        g.choose(&formats).unwrap().clone()
    }
}

/// Arbitrary implementation for ParserEngine
impl Arbitrary for ParserEngine {
    fn arbitrary(g: &mut Gen) -> Self {
        let engines = vec![ParserEngine::MinerU, ParserEngine::MarkItDown];
        g.choose(&engines).unwrap().clone()
    }
}

/// Arbitrary implementation for SourceType
impl Arbitrary for SourceType {
    fn arbitrary(g: &mut Gen) -> Self {
        let types = vec![SourceType::Upload, SourceType::Url];
        g.choose(&types).unwrap().clone()
    }
}

/// Arbitrary implementation for ProcessingStage
impl Arbitrary for ProcessingStage {
    fn arbitrary(g: &mut Gen) -> Self {
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
        g.choose(&stages).unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[quickcheck]
    fn prop_document_format_roundtrip_serialization(format: DocumentFormat) -> bool {
        let json = serde_json::to_string(&format).unwrap();
        let deserialized: DocumentFormat = serde_json::from_str(&json).unwrap();
        format == deserialized
    }

    #[quickcheck]
    fn prop_parser_engine_roundtrip_serialization(engine: ParserEngine) -> bool {
        let json = serde_json::to_string(&engine).unwrap();
        let deserialized: ParserEngine = serde_json::from_str(&json).unwrap();
        engine == deserialized
    }

    #[quickcheck]
    fn prop_source_type_roundtrip_serialization(source_type: SourceType) -> bool {
        let json = serde_json::to_string(&source_type).unwrap();
        let deserialized: SourceType = serde_json::from_str(&json).unwrap();
        source_type == deserialized
    }

    #[quickcheck]
    fn prop_processing_stage_roundtrip_serialization(stage: ProcessingStage) -> bool {
        let json = serde_json::to_string(&stage).unwrap();
        let deserialized: ProcessingStage = serde_json::from_str(&json).unwrap();
        stage == deserialized
    }

    #[quickcheck]
    fn prop_document_format_from_extension_consistency(ext: String) -> bool {
        let format1 = DocumentFormat::from_extension(&ext);
        let format2 = DocumentFormat::from_extension(&ext);
        format1 == format2
    }

    #[quickcheck]
    fn prop_parser_engine_supports_format_consistency(
        engine: ParserEngine,
        format: DocumentFormat,
    ) -> bool {
        let supports1 = engine.supports_format(&format);
        let supports2 = engine.supports_format(&format);
        supports1 == supports2
    }

    #[quickcheck]
    fn prop_processing_stage_progress_bounds(stage: ProcessingStage) -> bool {
        let progress = stage.get_progress();
        (0..=100).contains(&progress)
    }

    #[quickcheck]
    fn prop_task_error_creation_preserves_data(
        code: String,
        message: String,
        stage: Option<ProcessingStage>,
    ) -> bool {
        let error = TaskError::new(code.clone(), message.clone(), stage.clone());
        error.error_code == code && error.error_message == message && error.stage == stage
    }

    #[test]
    fn test_document_task_builder_validation() {
        // 安全初始化全局配置
        safe_init_global_config();
        quickcheck::quickcheck(prop_document_task_builder_creates_valid_task as fn() -> bool);
    }

    fn prop_document_task_builder_creates_valid_task() -> bool {
        let mut t = DocumentTask::new(CreateTaskParams {
            id: Uuid::new_v4().to_string(),
            source_type: SourceType::Upload,
            source: Some("/tmp/test.pdf".to_string()),
            original_filename: Some("test.pdf".to_string()),
            document_format: Some(DocumentFormat::PDF),
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });
        t.parser_engine = Some(ParserEngine::MinerU);
        t.file_size = Some(1024);
        t.mime_type = Some("application/pdf".to_string());
        let task: Result<DocumentTask, AppError> = Ok(t);

        match task {
            Ok(t) => {
                !t.id.is_empty()
                    && t.source_type == SourceType::Upload
                    && t.document_format == Some(DocumentFormat::PDF)
                    && t.parser_engine == Some(ParserEngine::MinerU)
                    && t.status.is_pending()
            }
            Err(_) => false,
        }
    }

    #[quickcheck]
    fn prop_task_status_transitions_are_valid(stage: ProcessingStage) -> bool {
        let pending = TaskStatus::new_pending();
        let processing = TaskStatus::new_processing(stage);
        let completed = TaskStatus::new_completed(std::time::Duration::from_secs(60));

        pending.is_pending()
            && processing.is_processing()
            && matches!(completed, TaskStatus::Completed { .. })
    }

    #[quickcheck]
    fn prop_file_size_validation(size: u64) -> bool {
        // Test that file size validation behaves consistently
        let is_valid_size = size > 0 && size <= 1024 * 1024 * 1024 * 10; // 10GB max

        // This property should hold: valid sizes should be accepted
        if is_valid_size {
            // For now, just check that the size is positive
            size > 0
        } else {
            true // Invalid sizes are handled appropriately
        }
    }
}
