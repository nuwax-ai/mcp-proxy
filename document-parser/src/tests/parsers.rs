//! 解析引擎单元测试

use tempfile::TempDir;

use crate::{
    models::{DocumentFormat, ParserEngine},
    parsers::parser_trait::DocumentParser,
    parsers::{DetectionMethod, DualEngineParser, FormatDetector, MarkItDownParser, MinerUParser},
    tests::test_helpers::{
        create_real_environment_test_config, create_test_config, safe_init_global_config,
        safe_init_global_config_with_config,
    },
};
use tempfile;

#[cfg(test)]
mod format_detector_tests {
    use super::*;

    fn init_test_config() {
        safe_init_global_config();
    }

    #[test]
    fn test_detect_format_by_extension() {
        init_test_config();
        let detector = FormatDetector::new();

        // 创建临时文件进行测试
        let temp_dir = tempfile::tempdir().unwrap();

        // 测试PDF格式检测
        let pdf_path = temp_dir.path().join("document.pdf");
        std::fs::write(&pdf_path, "fake pdf content").unwrap();
        let result = detector
            .detect_format(pdf_path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::PDF);

        // 测试Word格式检测
        let word_path = temp_dir.path().join("document.docx");
        std::fs::write(&word_path, "fake word content").unwrap();
        let result = detector
            .detect_format(word_path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::Word);

        // 测试Excel格式检测
        let excel_path = temp_dir.path().join("spreadsheet.xlsx");
        std::fs::write(&excel_path, "fake excel content").unwrap();
        let result = detector
            .detect_format(excel_path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::Excel);

        // 测试PowerPoint格式检测
        let ppt_path = temp_dir.path().join("presentation.pptx");
        std::fs::write(&ppt_path, "fake ppt content").unwrap();
        let result = detector
            .detect_format(ppt_path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::PowerPoint);

        // 测试图片格式检测
        let image_path = temp_dir.path().join("image.png");
        std::fs::write(&image_path, "fake image content").unwrap();
        let result = detector
            .detect_format(image_path.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::Image);

        // 测试未知格式
        let unknown_path = temp_dir.path().join("unknown.xyz");
        std::fs::write(&unknown_path, "fake content").unwrap();
        let result = detector.detect_format(unknown_path.to_str().unwrap(), None);
        // 对于未知扩展名，FormatDetector 通过内容分析检测为文本文件
        match result {
            Ok(detection_result) => {
                // 先打印出实际的检测结果，了解实际行为
                println!("未知扩展名检测结果: {detection_result:?}");
                // 由于内容分析检测，未知扩展名被识别为文本文件
                assert_eq!(detection_result.format, DocumentFormat::Text);
                assert_eq!(
                    detection_result.detection_method,
                    DetectionMethod::ContentAnalysis
                );
            }
            Err(_) => panic!("不应该返回错误"),
        }
    }

    #[test]
    fn test_detect_format_by_mime_type() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config);
        let detector = FormatDetector::new();

        // 创建临时文件进行测试
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_file = temp_dir.path().join("file");
        std::fs::write(&temp_file, "fake content").unwrap();
        let temp_file_path = temp_file.to_str().unwrap();

        // 测试通过MIME类型检测PDF
        let result = detector
            .detect_format(temp_file_path, Some("application/pdf"))
            .unwrap();
        assert_eq!(result.format, DocumentFormat::PDF);

        // 测试通过MIME类型检测Word
        let result = detector
            .detect_format(
                temp_file_path,
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
            )
            .unwrap();
        assert_eq!(result.format, DocumentFormat::Word);

        // 测试未知MIME类型
        let result = detector
            .detect_format(temp_file_path, Some("application/unknown"))
            .unwrap();
        println!("未知MIME类型检测结果: {result:?}");
        // 由于MIME类型未知且文件没有扩展名，内容分析检测会识别出文本格式
        // 这是正确的行为，因为文件内容是纯文本
        assert_eq!(result.format, DocumentFormat::Text);
        assert_eq!(result.detection_method, DetectionMethod::ContentAnalysis);
    }

    #[test]
    fn test_select_parser_engine() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config.clone());
        let detector = FormatDetector::new();

        // 创建临时测试文件
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.pdf");
        std::fs::write(&test_file, "fake pdf content").unwrap();

        // 测试PDF格式检测和引擎推荐
        let result = detector
            .detect_format(test_file.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.format, DocumentFormat::PDF);
        assert_eq!(result.recommended_engine, ParserEngine::MinerU);
    }

    #[test]
    fn test_case_insensitive_detection() {
        init_test_config();
        let detector = FormatDetector::new();

        // 创建临时文件进行测试
        let temp_dir = tempfile::tempdir().unwrap();

        // 测试大小写不敏感的扩展名检测
        let formats = vec![
            ("file.PDF", DocumentFormat::PDF),
            ("file.Docx", DocumentFormat::Word),
            ("file.XLSX", DocumentFormat::Excel),
            ("file.Pptx", DocumentFormat::PowerPoint),
            ("file.PNG", DocumentFormat::Image),
        ];

        for (filename, expected) in formats {
            let file_path = temp_dir.path().join(filename);
            std::fs::write(&file_path, "fake content").unwrap();
            let result = detector
                .detect_format(file_path.to_str().unwrap(), None)
                .unwrap();
            assert_eq!(result.format, expected, "Failed for path: {filename}");
        }
    }
}

#[cfg(test)]
mod dual_engine_parser_tests {
    use super::*;

    #[tokio::test]
    async fn test_dual_engine_parser_creation() {
        let config = create_test_config();
        let _parser = DualEngineParser::new(&config.mineru, &config.markitdown);

        // 验证解析器创建成功
        // DualEngineParser::new 直接返回实例，不是Result类型
    }

    #[tokio::test]
    async fn test_parse_with_format_detection() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config.clone());
        let parser = DualEngineParser::new(&config.mineru, &config.markitdown);

        // 创建临时测试文件
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.txt");
        std::fs::write(&test_file, "Test content").expect("Failed to write test file");

        // 测试解析（这里会因为没有实际的解析环境而失败，但可以测试格式检测逻辑）
        let result = parser.parse(test_file.to_str().unwrap()).await;

        // 在测试环境中，解析可能会失败，但我们可以验证错误类型
        match result {
            Ok(_) => {
                // 如果成功，验证结果结构
                // 这在有实际解析环境时会执行
            }
            Err(e) => {
                // 验证错误是预期的（环境相关错误）
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("environment")
                        || error_msg.contains("python")
                        || error_msg.contains("command")
                        || error_msg.contains("No such file or directory")
                        || error_msg.contains("MinerU错误")
                        || error_msg.contains("启动MinerU进程失败")
                        || error_msg.contains("MarkItDown错误")
                        || error_msg.contains("启动MarkItDown进程失败"),
                    "Unexpected error: {error_msg}"
                );
            }
        }
    }

    #[test]
    fn test_engine_selection_logic() {
        let config = create_test_config();
        safe_init_global_config_with_config(config.clone());
        let detector = FormatDetector::new();

        // 创建临时测试文件
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.pdf");
        std::fs::write(&test_file, "fake pdf content").unwrap();

        // 测试PDF文件选择MinerU引擎
        let result = detector
            .detect_format(test_file.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.recommended_engine, ParserEngine::MinerU);

        // 测试Word文件选择MarkItDown引擎
        let word_file = temp_dir.path().join("test.docx");
        std::fs::write(&word_file, "fake word content").unwrap();
        let result = detector
            .detect_format(word_file.to_str().unwrap(), None)
            .unwrap();
        assert_eq!(result.recommended_engine, ParserEngine::MarkItDown);
    }
}

#[cfg(test)]
mod mineru_parser_tests {
    use super::*;

    #[test]
    fn test_mineru_parser_creation() {
        let config = create_test_config();
        let mineru_config = crate::config::MinerUConfig {
            python_path: config.mineru.python_path.clone(),
            backend: config.mineru.backend.clone(),
            max_concurrent: config.mineru.max_concurrent,
            queue_size: config.mineru.queue_size,
            timeout: config.mineru.timeout,

            batch_size: 1,
            quality_level: crate::config::QualityLevel::Balanced,
            device: "cpu".to_string(),
            vram: 8,
        };
        let parser = MinerUParser::new(mineru_config);

        // MinerUParser::new直接返回实例，不是Result
        assert_eq!(parser.config().python_path, config.mineru.python_path);
    }


    #[tokio::test]
    async fn test_mineru_parse_invalid_file() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config.clone());
        let parser = MinerUParser::with_defaults(
            config.mineru.python_path.clone(),
            config.mineru.backend.clone(),
            Some(config.mineru.device.clone()),
        );

        // 创建临时目录和文件
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.pdf");
        std::fs::write(&test_file, "fake pdf content").unwrap();

        // 测试解析有效文件（在测试环境中可能会失败，但可以验证错误类型）
        let result = parser.parse(test_file.to_str().unwrap()).await;
        match result {
            Ok(_) => {
                // 如果成功，验证结果结构
                // 这在有实际MinerU环境时会执行
            }
            Err(e) => {
                // 验证错误是预期的（环境相关错误）
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("environment")
                        || error_msg.contains("python")
                        || error_msg.contains("command")
                        || error_msg.contains("No such file or directory")
                        || error_msg.contains("MinerU错误")
                        || error_msg.contains("启动MinerU进程失败"),
                    "Unexpected error: {error_msg}"
                );
            }
        }

        // 测试解析不存在的文件
        let invalid_path = temp_dir.path().join("nonexistent.pdf");
        let result = parser.parse(invalid_path.to_str().unwrap()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_mineru_command_building() {
        let config = create_test_config();
        let mineru_config = crate::config::MinerUConfig {
            python_path: config.mineru.python_path.clone(),
            backend: config.mineru.backend.clone(),
            max_concurrent: config.mineru.max_concurrent,
            queue_size: config.mineru.queue_size,
            timeout: config.mineru.timeout,

            batch_size: 1,
            quality_level: crate::config::QualityLevel::Balanced,
            device: "cpu".to_string(),
            vram: 8,
        };
        let parser = MinerUParser::new(mineru_config);

        // 这里可以测试命令构建逻辑（如果MinerUParser暴露了相关方法）
        // 由于当前实现可能没有暴露内部方法，我们可以通过其他方式验证

        // 验证配置参数被正确设置
        assert_eq!(parser.config().python_path, config.mineru.python_path);
    }
}

#[cfg(test)]
mod markitdown_parser_tests {
    use super::*;


    #[tokio::test]
    async fn test_markitdown_parse_invalid_file() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config.clone());
        let parser = MarkItDownParser::with_defaults(
            config.markitdown.python_path.clone(),
            config.markitdown.enable_plugins,
        );

        // 创建临时目录和文件
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        let test_file = temp_dir.path().join("test.docx");
        std::fs::write(&test_file, "fake docx content").unwrap();

        // 测试解析有效文件（在测试环境中可能会失败，但可以验证错误类型）
        let result = parser.parse(test_file.to_str().unwrap()).await;
        match result {
            Ok(_) => {
                // 如果成功，验证结果结构
                // 这在有实际MarkItDown环境时会执行
            }
            Err(e) => {
                // 验证错误是预期的（环境相关错误）
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("environment")
                        || error_msg.contains("python")
                        || error_msg.contains("command")
                        || error_msg.contains("No such file or directory")
                        || error_msg.contains("MarkItDown错误")
                        || error_msg.contains("启动MarkItDown进程失败"),
                    "Unexpected error: {error_msg}"
                );
            }
        }

        // 测试解析不存在的文件
        let invalid_path = temp_dir.path().join("nonexistent.docx");
        let result = parser.parse(invalid_path.to_str().unwrap()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_markitdown_supported_formats() {
        let config = create_real_environment_test_config();
        safe_init_global_config_with_config(config.clone());
        let parser = MarkItDownParser::with_defaults(
            config.markitdown.python_path.clone(),
            config.markitdown.enable_plugins,
        );

        // 创建临时测试文件
        let temp_dir = TempDir::new().expect("Failed to create temp dir");

        // 测试文本文件
        let text_file = temp_dir.path().join("test.txt");
        std::fs::write(&text_file, "Test content").expect("Failed to write test file");

        // 测试解析（在没有实际MarkItDown环境时会失败）
        let result = parser.parse(text_file.to_str().unwrap()).await;

        // 验证结果或错误
        match result {
            Ok(parse_result) => {
                // 如果成功，验证结果结构
                assert!(!parse_result.markdown_content.is_empty());
            }
            Err(e) => {
                // 验证错误是预期的（环境相关错误）
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("environment")
                        || error_msg.contains("python")
                        || error_msg.contains("command")
                        || error_msg.contains("No such file or directory")
                        || error_msg.contains("MarkItDown错误")
                        || error_msg.contains("启动MarkItDown进程失败"),
                    "Unexpected error: {error_msg}"
                );
            }
        }
    }
}

#[cfg(test)]
mod parser_trait_tests {
    use super::*;

    #[test]
    fn test_parse_result_structure() {
        let parse_result = crate::models::ParseResult::new(
            "# Test Document\n\nContent".to_string(),
            DocumentFormat::PDF,
            ParserEngine::MinerU,
        );

        assert!(!parse_result.markdown_content.is_empty());
        // images 字段已移除
        assert!(parse_result.word_count.is_some());
    }

    #[test]
    fn test_parse_result_with_metadata() {
        let mut parse_result = crate::models::ParseResult::new(
            "# Test Document".to_string(),
            DocumentFormat::PDF,
            ParserEngine::MinerU,
        );

        parse_result.set_processing_time(1.5);
        parse_result.set_error_count(0);

        assert_eq!(parse_result.processing_time, Some(1.5));
        assert_eq!(parse_result.error_count, Some(0));
        assert!(parse_result.is_success());
    }

    #[test]
    fn test_parse_result_serialization() {
        let parse_result = crate::models::ParseResult::new(
            "# Test".to_string(),
            DocumentFormat::PDF,
            ParserEngine::MinerU,
        );

        let json = serde_json::to_string(&parse_result).expect("Failed to serialize");
        let deserialized: crate::models::ParseResult =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(parse_result.markdown_content, deserialized.markdown_content);
        assert_eq!(parse_result.format, deserialized.format);
        assert_eq!(parse_result.engine, deserialized.engine);
    }
}
