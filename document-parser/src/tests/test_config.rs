//! Test configuration and setup utilities
//!
//! This module provides comprehensive test configuration and setup utilities
//! for running tests with proper isolation and cleanup.

use std::sync::Once;
use tempfile::TempDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

static INIT: Once = Once::new();

/// Initialize test logging (call once per test run)
pub fn init_test_logging() {
    INIT.call_once(|| {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "document_parser=debug,tower_http=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer().with_test_writer())
            .init();
    });
}

/// Test environment configuration
pub struct TestEnvironment {
    pub temp_dir: TempDir,
    pub db_path: String,
    pub config: crate::config::AppConfig,
}

impl TestEnvironment {
    /// Create a new isolated test environment
    pub fn new() -> Self {
        init_test_logging();

        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let db_path = temp_dir
            .path()
            .join("test.db")
            .to_string_lossy()
            .to_string();

        let config = crate::config::AppConfig {
            environment: "test".to_string(),
            server: crate::config::ServerConfig {
                port: 0, // Use random port for tests
                host: "127.0.0.1".to_string(),
            },
            log: crate::config::LogConfig {
                level: "debug".to_string(),
                path: temp_dir
                    .path()
                    .join("test.log")
                    .to_string_lossy()
                    .to_string(),
                retain_days: 20,
            },
            document_parser: crate::config::DocumentParserConfig {
                max_concurrent: 2,
                queue_size: 10,
                download_timeout: 30,
                processing_timeout: 300,
            },
            file_size_config: {
                // 从配置文件加载文件大小配置，而不是使用默认值
                match crate::config::AppConfig::load_base_config() {
                    Ok(base_config) => base_config.file_size_config,
                    Err(_) => {
                        // 如果加载失败，使用测试专用的配置（与config.yml中的值一致）
                        crate::config::GlobalFileSizeConfig {
                            max_file_size: crate::config::FileSize::from_mb(100), // 100MB
                            large_document_threshold: crate::config::FileSize::from_mb(50), // 50MB
                        }
                    }
                }
            },
            storage: crate::config::StorageConfig {
                sled: crate::config::SledConfig {
                    path: db_path.clone(),
                    cache_capacity: 1024 * 1024,
                },
                oss: crate::config::OssConfig {
                    endpoint: "https://test-endpoint.com".to_string(),
                    public_bucket: "test-public-bucket".to_string(),
                    private_bucket: "test-private-bucket".to_string(),
                    access_key_id: "test-key".to_string(),
                    access_key_secret: "test-secret".to_string(),
                    upload_directory: "test".to_string(),
                    region: "oss-rg-china-mainland".to_string(),
                },
            },
            external_integration: crate::config::ExternalIntegrationConfig {
                webhook_url: "https://test-webhook.com".to_string(),
                api_key: "test-api-key".to_string(),
                timeout: 30,
            },
            mineru: crate::config::MinerUConfig {
                backend: "pipeline".to_string(),
                python_path: "python3".to_string(),
                max_concurrent: 1,
                queue_size: 5,
                timeout: 0, // 使用统一超时配置
                batch_size: 1,
                quality_level: crate::config::QualityLevel::Balanced,
                device: "cpu".to_string(),
                vram: 8,
            },
            markitdown: crate::config::MarkItDownConfig {
                python_path: "python3".to_string(),
                timeout: 0, // 使用统一超时配置
                enable_plugins: false,
                features: crate::config::MarkItDownFeatures {
                    ocr: false,
                    audio_transcription: false,
                    azure_doc_intel: false,
                    youtube_transcription: false,
                },
            },
        };

        Self {
            temp_dir,
            db_path,
            config,
        }
    }

    /// Get the temporary directory path
    pub fn temp_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }

    /// Create a test file with given content
    pub fn create_test_file(&self, filename: &str, content: &[u8]) -> std::path::PathBuf {
        let file_path = self.temp_dir.path().join(filename);
        std::fs::write(&file_path, content).expect("Failed to create test file");
        file_path
    }

    /// Create a test PDF file
    pub fn create_test_pdf(&self, filename: &str) -> std::path::PathBuf {
        // Create a minimal PDF-like file for testing
        let pdf_content = b"%PDF-1.4\n1 0 obj\n<<\n/Type /Catalog\n/Pages 2 0 R\n>>\nendobj\n2 0 obj\n<<\n/Type /Pages\n/Kids [3 0 R]\n/Count 1\n>>\nendobj\n3 0 obj\n<<\n/Type /Page\n/Parent 2 0 R\n/MediaBox [0 0 612 792]\n>>\nendobj\nxref\n0 4\n0000000000 65535 f \n0000000009 00000 n \n0000000074 00000 n \n0000000120 00000 n \ntrailer\n<<\n/Size 4\n/Root 1 0 R\n>>\nstartxref\n179\n%%EOF";
        self.create_test_file(filename, pdf_content)
    }

    /// Create a test markdown file
    pub fn create_test_markdown(&self, filename: &str) -> std::path::PathBuf {
        let markdown_content = r#"# Test Document

This is a test document for testing purposes.

## Section 1

Content for section 1.

### Subsection 1.1

Content for subsection 1.1.

## Section 2

Content for section 2.

![Test Image](test-image.png)

[Test Link](https://example.com)
"#;
        self.create_test_file(filename, markdown_content.as_bytes())
    }

    /// Create a test image file
    pub fn create_test_image(&self, filename: &str) -> std::path::PathBuf {
        // Create a minimal PNG-like file for testing
        let png_header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\tpHYs\x00\x00\x0b\x13\x00\x00\x0b\x13\x01\x00\x9a\x9c\x18\x00\x00\x00\nIDATx\x9cc\xf8\x00\x00\x00\x01\x00\x01\x00\x00\x00\x00IEND\xaeB`\x82";
        self.create_test_file(filename, png_header)
    }
}

/// Test data generators
pub mod generators {
    use crate::models::*;
    use crate::tests::test_helpers::safe_init_global_config;
    use uuid::Uuid;

    /// Generate a test DocumentTask with default values
    pub fn test_document_task() -> DocumentTask {
        // 安全初始化全局配置
        safe_init_global_config();
        {
            let mut t = DocumentTask::new(
                Uuid::new_v4().to_string(),
                SourceType::Upload,
                Some("/tmp/test.pdf".to_string()),
                Some("test.pdf".to_string()),
                Some(DocumentFormat::PDF),
                Some("pipeline".to_string()),
                Some(24),
                Some(3),
            );
            t.parser_engine = Some(ParserEngine::MinerU);
            t.file_size = Some(1024 * 1024);
            t.mime_type = Some("application/pdf".to_string());
            t
        }
    }

    /// Generate a test DocumentTask with custom parameters
    pub fn test_document_task_with_params(
        source_type: SourceType,
        format: DocumentFormat,
        engine: ParserEngine,
    ) -> DocumentTask {
        // 安全初始化全局配置
        safe_init_global_config();
        {
            let mut t = DocumentTask::new(
                Uuid::new_v4().to_string(),
                source_type,
                Some("/tmp/test.pdf".to_string()),
                Some("test.pdf".to_string()),
                Some(format),
                Some("pipeline".to_string()),
                Some(24),
                Some(3),
            );
            t.parser_engine = Some(engine);
            t.file_size = Some(1024 * 1024);
            t.mime_type = Some("application/pdf".to_string());
            t
        }
    }

    /// Generate test markdown content with various structures
    pub fn test_markdown_samples() -> Vec<(&'static str, &'static str)> {
        vec![
            ("simple", "# Title\nContent here."),
            (
                "nested",
                "# Chapter 1\n## Section 1.1\n### Subsection 1.1.1\n## Section 1.2\n# Chapter 2",
            ),
            ("empty", ""),
            ("no_headers", "Just content without any headers."),
            (
                "unicode",
                "# 中文标题\n中文内容测试。\n## English Section\nMixed content.",
            ),
            (
                "with_images",
                "# Document\n![Image](image.png)\nContent with image.",
            ),
            (
                "with_links",
                "# Document\n[Link](https://example.com)\nContent with link.",
            ),
            (
                "complex",
                r#"# Main Title

Introduction paragraph.

## Chapter 1: Getting Started

This chapter covers the basics.

### 1.1 Installation

Installation instructions here.

### 1.2 Configuration

Configuration details here.

## Chapter 2: Advanced Topics

Advanced content here.

![Diagram](diagram.png)

### 2.1 Performance

Performance considerations.

### 2.2 Security

Security best practices.

## Conclusion

Final thoughts.
"#,
            ),
        ]
    }

    /// Generate test error scenarios
    pub fn test_error_scenarios() -> Vec<TaskError> {
        vec![
            TaskError::new(
                "E001".to_string(),
                "File not found".to_string(),
                Some(ProcessingStage::DownloadingDocument),
            ),
            TaskError::new(
                "E002".to_string(),
                "Invalid file format".to_string(),
                Some(ProcessingStage::FormatDetection),
            ),
            TaskError::new(
                "E003".to_string(),
                "Parser execution failed".to_string(),
                Some(ProcessingStage::MinerUExecuting),
            ),
            TaskError::new("E004".to_string(), "Network timeout".to_string(), None),
            TaskError::new(
                "E005".to_string(),
                "Insufficient disk space".to_string(),
                Some(ProcessingStage::UploadingMarkdown),
            ),
        ]
    }
}

/// Test assertions and utilities
pub mod assertions {
    use crate::models::*;

    /// Assert that a task is in a valid state
    pub fn assert_valid_task(task: &DocumentTask) {
        assert!(!task.id.is_empty());
        assert!(uuid::Uuid::parse_str(&task.id).is_ok());
        assert!(task.created_at <= task.updated_at);
        assert!(task.updated_at <= task.expires_at);
        assert!(task.retry_count <= task.max_retries);

        // Validate status consistency
        match &task.status {
            TaskStatus::Pending { queued_at: _ } => {
                assert_eq!(task.progress, 0);
                assert!(task.error_message.is_none());
            }
            TaskStatus::Processing { .. } => {
                assert!(task.progress > 0 && task.progress < 100);
            }
            TaskStatus::Completed { .. } => {
                assert_eq!(task.progress, 100);
                assert!(task.error_message.is_none());
            }
            TaskStatus::Failed { .. } => {
                assert!(task.error_message.is_some());
            }
            TaskStatus::Cancelled { .. } => {
                // Cancelled tasks can have any progress
            }
        }
    }

    /// Assert that a structured document is valid
    pub fn assert_valid_structured_document(doc: &StructuredDocument) {
        assert!(!doc.task_id.is_empty());
        assert!(!doc.document_title.is_empty());
        assert_eq!(doc.toc.len(), doc.total_sections);

        // Validate TOC structure
        for section in &doc.toc {
            assert_valid_structured_section(section);
        }
    }

    /// Assert that a structured section is valid
    pub fn assert_valid_structured_section(section: &StructuredSection) {
        assert!(!section.id.is_empty());
        assert!(!section.title.is_empty());
        assert!(section.level > 0);

        // Validate children
        for child in &section.children {
            assert!(child.level > section.level);
            assert_valid_structured_section(child);
        }

        // Validate position information if present
        if let (Some(start), Some(end)) = (section.start_pos, section.end_pos) {
            assert!(start <= end);
        }
    }

    /// Assert that an error is properly formatted
    pub fn assert_valid_task_error(error: &TaskError) {
        assert!(!error.error_code.is_empty());
        assert!(!error.error_message.is_empty());
        // TaskError doesn't have timestamp field, so we skip this assertion
    }
}

/// Performance testing utilities
pub mod performance {
    use std::time::{Duration, Instant};

    /// Measure execution time of a function
    pub async fn measure_async<F, Fut, T>(f: F) -> (T, Duration)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let start = Instant::now();
        let result = f().await;
        let duration = start.elapsed();
        (result, duration)
    }

    /// Assert that an operation completes within a time limit
    pub fn assert_within_time_limit<T>(
        result: (T, Duration),
        limit: Duration,
        operation_name: &str,
    ) -> T {
        let (value, duration) = result;
        assert!(
            duration <= limit,
            "{operation_name} took {duration:?}, expected <= {limit:?}"
        );
        value
    }

    /// Benchmark configuration
    pub struct BenchmarkConfig {
        pub iterations: usize,
        pub warmup_iterations: usize,
        pub time_limit: Duration,
    }

    impl Default for BenchmarkConfig {
        fn default() -> Self {
            Self {
                iterations: 100,
                warmup_iterations: 10,
                time_limit: Duration::from_secs(1),
            }
        }
    }
}

// Re-export submodules for external use

#[cfg(test)]
mod test_config_tests {
    use super::*;
    use crate::models::*;

    #[test]
    #[ignore = "Uses global Once instance for tracing, fails when other tests poison it"]
    fn test_environment_creation() {
        let env = TestEnvironment::new();

        assert!(env.temp_path().exists());
        assert!(!env.db_path.is_empty());
        assert_eq!(env.config.server.host, "127.0.0.1");
        assert_eq!(env.config.document_parser.max_concurrent, 2);
    }

    #[test]
    #[ignore = "Uses global Once instance for tracing, fails when other tests poison it"]
    fn test_file_creation() {
        let env = TestEnvironment::new();

        let test_file = env.create_test_file("test.txt", b"test content");
        assert!(test_file.exists());

        let content = std::fs::read(&test_file).expect("Failed to read test file");
        assert_eq!(content, b"test content");
    }

    #[test]
    #[ignore = "Uses global Once instance for tracing, fails when other tests poison it"]
    fn test_pdf_creation() {
        let env = TestEnvironment::new();

        let pdf_file = env.create_test_pdf("test.pdf");
        assert!(pdf_file.exists());

        let content = std::fs::read(&pdf_file).expect("Failed to read PDF file");
        assert!(content.starts_with(b"%PDF"));
    }

    #[test]
    #[ignore = "Uses global Once instance for tracing, fails when other tests poison it"]
    fn test_markdown_creation() {
        let env = TestEnvironment::new();

        let md_file = env.create_test_markdown("test.md");
        assert!(md_file.exists());

        let content = std::fs::read_to_string(&md_file).expect("Failed to read markdown file");
        assert!(content.contains("# Test Document"));
        assert!(content.contains("## Section 1"));
    }

    #[test]
    fn test_generators() {
        let task = generators::test_document_task();
        assertions::assert_valid_task(&task);

        let custom_task = generators::test_document_task_with_params(
            SourceType::Url,
            DocumentFormat::Word,
            ParserEngine::MarkItDown,
        );
        assert_eq!(custom_task.source_type, SourceType::Url);
        assert_eq!(custom_task.document_format, Some(DocumentFormat::Word));
        assert_eq!(custom_task.parser_engine, Some(ParserEngine::MarkItDown));
    }

    #[test]
    fn test_markdown_samples() {
        let samples = generators::test_markdown_samples();
        assert!(!samples.is_empty());

        for (name, _content) in samples {
            assert!(!name.is_empty());
            // Content can be empty for the "empty" test case
        }
    }

    #[test]
    fn test_error_scenarios() {
        let errors = generators::test_error_scenarios();
        assert!(!errors.is_empty());

        for error in errors {
            assertions::assert_valid_task_error(&error);
        }
    }
}
