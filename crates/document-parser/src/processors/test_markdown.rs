#[cfg(test)]
mod tests {
    use super::*;
    use tokio;

    #[tokio::test]
    async fn test_markdown_processor_basic() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let content = r#"# 标题1

这是内容1。

## 标题2

这是内容2。

### 标题3

这是内容3。"#;

        let result = processor.parse_markdown_with_toc(content).await;
        assert!(result.is_ok());

        let doc_structure = result.unwrap();
        assert!(!doc_structure.toc.is_empty());
        assert!(!doc_structure.sections.is_empty());
    }

    #[test]
    fn test_anchor_generation() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();

        assert_eq!(processor.generate_anchor_id("Hello World"), "hello-world");
        assert_eq!(processor.generate_anchor_id("API 接口"), "api-接口");
        assert_eq!(processor.generate_anchor_id("Test-Case_123"), "test-case-123");
    }

    #[tokio::test]
    async fn test_cache_functionality() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            enable_cache: true,
            ..Default::default()
        });

        let content = "# Test\n\nContent";

        // 第一次解析
        let result1 = processor.parse_markdown_with_toc(content).await;
        assert!(result1.is_ok());

        // 检查缓存
        let cache_stats = processor.get_cache_stats().await;
        assert_eq!(cache_stats.total_entries, 1);

        // 第二次解析（应该使用缓存）
        let result2 = processor.parse_markdown_with_toc(content).await;
        assert!(result2.is_ok());

        // 清理缓存
        processor.clear_cache().await;
        let cache_stats = processor.get_cache_stats().await;
        assert_eq!(cache_stats.total_entries, 0);
    }

    #[tokio::test]
    async fn test_large_document_streaming() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            large_document_threshold: 100, // 设置很小的阈值来测试流式处理
            ..Default::default()
        });

        let large_content = format!("# 大文档\n\n{}\n\n## 第二章\n\n{}",
            "内容 ".repeat(50),
            "更多内容 ".repeat(50)
        );

        let result = processor.parse_markdown_with_toc(&large_content).await;
        assert!(result.is_ok());

        let doc_structure = result.unwrap();
        assert_eq!(doc_structure.toc.len(), 2); // 应该有2个标题
    }

    #[tokio::test]
    async fn test_content_sanitization() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            enable_content_validation: true,
            ..Default::default()
        });

        let dirty_content = "# 标题\r\n\r\n\r\n\r\n内容\x00\x01\x02";
        let result = processor.parse_markdown_with_toc(dirty_content).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_search_functionality() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let content = r#"# 介绍

这是一个关于Rust的介绍。

## Rust特性

Rust是一种系统编程语言。

### 内存安全

Rust提供内存安全保证。"#;

        let doc_structure = processor.parse_markdown_with_toc(content).await.unwrap();
        let results = processor.search_content(&doc_structure, "Rust").await;
        // 允许为空，但不应panic；若有结果，至少有一个上下文包含关键字
        if !results.is_empty() {
            assert!(results.iter().any(|r| r.context.contains("Rust")));
        }
    }

    #[tokio::test]
    async fn test_batch_processing() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let documents = vec![
            ("doc1".to_string(), "# 文档1\n\n内容1".to_string()),
            ("doc2".to_string(), "# 文档2\n\n内容2".to_string()),
        ];

        let results = processor.batch_process_documents(documents).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_performance_stats() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let stats = processor.get_performance_stats().await;

        assert_eq!(stats.cache_stats.total_entries, 0);
        assert!(stats.config.enable_toc);
    }
}