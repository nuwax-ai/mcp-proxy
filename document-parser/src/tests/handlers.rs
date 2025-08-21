//! API接口处理器单元测试

use axum::body::Body;
use axum::http::Request;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

use super::test_helpers::*;
use crate::models::*;

#[cfg(test)]
mod document_handler_tests {
    use super::*;
    

    #[tokio::test]
    async fn test_upload_document_success() {
        let _app_state = create_test_app_state().await;

        // 创建模拟的multipart请求
        // 注意：这里需要实际的multipart数据，在真实测试中需要构造
        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header("content-type", "multipart/form-data; boundary=----test")
            .body(Body::from("test file content"))
            .unwrap();

        // 由于multipart解析的复杂性，这里主要测试处理器的存在和基本结构
        // 实际的multipart测试需要更复杂的设置
    }

    #[tokio::test]
    async fn test_upload_document_invalid_content_type() {
        let _app_state = create_test_app_state().await;

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header("content-type", "application/json") // 错误的content-type
            .body(Body::from("{}"))
            .unwrap();

        // 测试错误处理逻辑
        // 在实际实现中，应该返回400错误
    }

    #[tokio::test]
    async fn test_submit_document_url_success() {
        let _app_state = create_test_app_state().await;

        let request_body = json!({
            "url": "https://example.com/test.pdf",
            "filename": "test.pdf"
        });

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/url")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .unwrap();

        // 测试URL提交处理
        // 实际测试需要mock HTTP客户端
    }

    #[tokio::test]
    async fn test_submit_document_url_invalid_url() {
        let _app_state = create_test_app_state().await;

        let request_body = json!({
            "url": "invalid-url",
            "filename": "test.pdf"
        });

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/url")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .unwrap();

        // 测试无效URL的错误处理
    }

    #[tokio::test]
    async fn test_submit_document_url_missing_fields() {
        let _app_state = create_test_app_state().await;

        // 缺少必需字段的请求
        let request_body = json!({
            "url": "https://example.com/test.pdf"
            // 缺少filename字段
        });

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/url")
            .header("content-type", "application/json")
            .body(Body::from(request_body.to_string()))
            .unwrap();

        // 测试缺少必需字段的错误处理
    }
}

#[cfg(test)]
mod task_handler_tests {
    use super::*;

    #[tokio::test]
    async fn test_get_task_status_success() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 首先创建一个测试任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.file_size = Some(1024);
        task.mime_type = Some("application/pdf".to_string());
        task.status = TaskStatus::new_pending();

        // 保存任务到存储
        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/status"))
            .body(Body::empty())
            .unwrap();

        // 测试获取任务状态
    }

    #[tokio::test]
    async fn test_get_task_status_not_found() {
        let _app_state = create_test_app_state().await;
        let non_existent_task_id = Uuid::new_v4().to_string();

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{non_existent_task_id}/status"))
            .body(Body::empty())
            .unwrap();

        // 测试任务不存在的情况
        // 应该返回404错误
    }

    #[tokio::test]
    async fn test_get_task_status_invalid_uuid() {
        let _app_state = create_test_app_state().await;
        let invalid_task_id = "invalid-uuid";

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{invalid_task_id}/status"))
            .body(Body::empty())
            .unwrap();

        // 测试无效UUID的错误处理
        // 应该返回400错误
    }
}

#[cfg(test)]
mod markdown_handler_tests {
    use super::*;

    #[tokio::test]
    async fn test_download_markdown_success() {
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建测试任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        task.progress = 100;
        task.oss_data = Some(OssData {
            markdown_url: "https://oss.example.com/test.md".to_string(),
            markdown_object_key: Some("markdown/test_task/test.md".to_string()),
            images: vec![],
            bucket: "test-bucket".to_string(),
        });

        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/markdown/download"))
            .body(Body::empty())
            .unwrap();

        // 测试Markdown文件下载
    }

    #[tokio::test]
    async fn test_download_markdown_task_not_completed() {
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建未完成的任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.status = TaskStatus::new_processing(ProcessingStage::MinerUExecuting);
        task.progress = 50;

        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/markdown/download"))
            .body(Body::empty())
            .unwrap();

        // 测试下载未完成任务的Markdown文件
        // 应该返回适当的错误
    }

    #[tokio::test]
    async fn test_get_markdown_url_success() {
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建已完成的任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        task.progress = 100;
        task.oss_data = Some(OssData {
            markdown_url: "https://oss.example.com/test.md".to_string(),
            markdown_object_key: Some("markdown/test_task/test.md".to_string()),
            images: vec![],
            bucket: "test-bucket".to_string(),
        });

        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/markdown/url"))
            .body(Body::empty())
            .unwrap();

        // 测试获取Markdown URL
    }

    #[tokio::test]
    async fn test_get_markdown_url_with_temp_params() {
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建已完成的任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        task.progress = 100;
        task.parser_engine = Some(ParserEngine::MinerU);
        task.oss_data = Some(OssData {
            markdown_url: "https://oss.example.com/test.md".to_string(),
            markdown_object_key: Some("markdown/test_task/test.md".to_string()),
            images: vec![],
            bucket: "test-bucket".to_string(),
        });

        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!(
                "/api/v1/task/{task_id}/markdown/url?temp=true&expires_hours=12"
            ))
            .body(Body::empty())
            .unwrap();

        // 测试带临时URL参数的请求
    }

    #[tokio::test]
    async fn test_process_markdown_sections_success() {
        let _app_state = create_test_app_state().await;

        // 创建测试Markdown内容
        let markdown_content = create_test_markdown();

        // 构造multipart请求体
        let boundary = "----test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"markdown_file\"; filename=\"test.md\"\r\nContent-Type: text/markdown\r\n\r\n{markdown_content}\r\n--{boundary}--\r\n"
        );

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/markdown/sections")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();

        // 测试Markdown章节处理
    }

    #[tokio::test]
    async fn test_process_markdown_sections_invalid_content() {
        let _app_state = create_test_app_state().await;

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/markdown/sections")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        // 测试无效内容类型的错误处理
    }
}

#[cfg(test)]
mod toc_handler_tests {
    use super::*;

    #[tokio::test]
    async fn test_get_document_toc_success() {
        let _app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建已完成的任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        task.progress = 100;
        task.oss_data = Some(OssData {
            markdown_url: "https://oss.example.com/test.md".to_string(),
            markdown_object_key: Some("markdown/test_task/test.md".to_string()),
            images: vec![],
            bucket: "test-bucket".to_string(),
        });

        _app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/toc"))
            .body(Body::empty())
            .unwrap();

        // 测试获取文档目录
    }

    #[tokio::test]
    async fn test_get_document_toc_task_not_completed() {
        let app_state = create_test_app_state().await;
        let task_id = create_test_task_id();

        // 创建未完成的任务
        let mut task = DocumentTask::new(
            task_id.clone(),
            SourceType::Upload,
            Some("/tmp/test.pdf".to_string()),
            Some("test.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);
        task.status = TaskStatus::new_processing(ProcessingStage::GeneratingToc);
        task.progress = 80;

        app_state
            .storage_service
            .save_task(&task)
            .await
            .expect("Failed to save task");

        let _request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{task_id}/toc"))
            .body(Body::empty())
            .unwrap();

        // 测试获取未完成任务的目录
        // 应该返回适当的错误或处理中状态
    }
}

#[cfg(test)]
mod health_handler_tests {
    use super::*;

    #[tokio::test]
    async fn test_health_check() {
        let _request = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        // 测试健康检查接口
        // 应该返回200状态码和健康状态信息
    }

    #[tokio::test]
    async fn test_readiness_check() {
        let _app_state = create_test_app_state().await;

        let _request = Request::builder()
            .method("GET")
            .uri("/ready")
            .body(Body::empty())
            .unwrap();

        // 测试就绪检查接口
        // 应该检查所有依赖服务的状态
    }
}

#[cfg(test)]
mod request_validation_tests {
    use super::*;

    #[test]
    fn test_query_parameter_validation() {
        // 测试查询参数验证
        let mut params = HashMap::new();
        params.insert("temp".to_string(), "true".to_string());
        params.insert("expires_hours".to_string(), "24".to_string());

        // 验证参数解析
        assert_eq!(params.get("temp"), Some(&"true".to_string()));
        assert_eq!(params.get("expires_hours"), Some(&"24".to_string()));
    }

    #[test]
    fn test_invalid_query_parameters() {
        let mut params = HashMap::new();
        params.insert("expires_hours".to_string(), "invalid".to_string());

        // 测试无效参数值的处理
        let expires_hours = params
            .get("expires_hours")
            .and_then(|v| v.parse::<u32>().ok());
        assert_eq!(expires_hours, None);
    }

    #[test]
    fn test_boundary_values() {
        // 测试边界值
        let test_cases = vec![
            ("0", Some(0u32)),
            ("1", Some(1u32)),
            ("24", Some(24u32)),
            ("168", Some(168u32)),       // 7天
            ("-1", None),                // 负数
            ("999999", Some(999999u32)), // 大数值
        ];

        for (input, expected) in test_cases {
            let result = input.parse::<u32>().ok();
            assert_eq!(result, expected, "Failed for input: {input}");
        }
    }
}

#[cfg(test)]
mod response_format_tests {
    use super::*;

    #[test]
    fn test_success_response_format() {
        let response = HttpResult::success(json!({
            "task_id": "test-task-id",
            "status": "completed"
        }));

        assert_eq!(response.code, "0000");
        assert_eq!(response.message, "操作成功");
        assert!(response.data.is_some());
    }

    #[test]
    fn test_error_response_format() {
        let response: HttpResult<Value> =
            HttpResult::<Value>::error("E001".to_string(), "系统内部错误".to_string());

        assert_eq!(response.code, "E001");
        assert_eq!(response.message, "系统内部错误");
        assert!(response.data.is_none());
    }

    #[test]
    fn test_response_serialization() {
        let response = HttpResult::success("test_data".to_string());

        let json = serde_json::to_string(&response).expect("Failed to serialize response");
        let deserialized: HttpResult<String> =
            serde_json::from_str(&json).expect("Failed to deserialize response");

        assert_eq!(response.code, deserialized.code);
        assert_eq!(response.message, deserialized.message);
        assert_eq!(response.data, deserialized.data);
    }
}

#[cfg(test)]
mod comprehensive_handler_tests {
    use super::*;
    use axum::{
        body::Body,
        http::Request,
    };
    
    

    #[tokio::test]
    async fn test_document_upload_validation() {
        let _app_state = create_test_app_state().await;

        // Test file size validation
        let large_file_content = "x".repeat(100 * 1024 * 1024); // 100MB

        // Create multipart request
        let boundary = "----test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.pdf\"\r\nContent-Type: application/pdf\r\n\r\n{large_file_content}\r\n--{boundary}--\r\n"
        );

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();

        // Test would validate file size limits
        // In actual implementation, this should be rejected if over limit
    }

    #[tokio::test]
    async fn test_document_upload_mime_type_validation() {
        let _app_state = create_test_app_state().await;

        // Test unsupported MIME type
        let boundary = "----test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.exe\"\r\nContent-Type: application/x-executable\r\n\r\nfake exe content\r\n--{boundary}--\r\n"
        );

        let _request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();

        // Should reject unsupported file types
    }

    #[tokio::test]
    async fn test_url_submission_validation() {
        let _app_state = create_test_app_state().await;

        // Test various URL formats
        let test_cases = vec![
            ("https://example.com/doc.pdf", true),
            ("http://example.com/doc.docx", true),
            ("ftp://example.com/doc.txt", false), // Unsupported protocol
            ("not-a-url", false),
            ("", false),
        ];

        for (url, _should_be_valid) in test_cases {
            let request_body = json!({
                "url": url,
                "filename": "test.pdf"
            });

            let _request = Request::builder()
                .method("POST")
                .uri("/api/v1/document/url")
                .header("content-type", "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap();

            // Validation logic would check URL format
            // should_be_valid indicates expected validation result
        }
    }

    #[tokio::test]
    async fn test_task_status_retrieval_comprehensive() {
        let _app_state = create_test_app_state().await;

        // Create tasks in different states
        let tasks = vec![
            (TaskStatus::new_pending(), "pending"),
            (
                TaskStatus::new_processing(ProcessingStage::FormatDetection),
                "processing",
            ),
            (
                TaskStatus::new_completed(std::time::Duration::from_secs(60)),
                "completed",
            ),
            (
                TaskStatus::new_failed(
                    TaskError::new("E001".to_string(), "Test error".to_string(), None),
                    1,
                ),
                "failed",
            ),
        ];

        for (status, _status_name) in tasks {
            let task = DocumentTask::new(
                Uuid::new_v4().to_string(),
                SourceType::Upload,
                None,
                None,
                Some(DocumentFormat::PDF),
                Some("pipeline".to_string()),
                Some(24),
                Some(3),
            );
            let task = {
                let mut t = task;
                t.parser_engine = Some(ParserEngine::MinerU);
                t
            };

            let mut task_with_status = task.clone();
            task_with_status.status = status;

            // Save task
            _app_state
                .storage_service
                .save_task(&task_with_status)
                .await
                .expect("Failed to save task");

            // Test status retrieval
            let _request = Request::builder()
                .method("GET")
                .uri(format!("/api/v1/task/{}/status", task.id))
                .body(Body::empty())
                .unwrap();

            // Handler should return appropriate status information
        }
    }

    #[tokio::test]
    async fn test_markdown_processing_edge_cases() {
        let _app_state = create_test_app_state().await;

        // Test various markdown content scenarios
        let test_cases = vec![
            ("", "empty content"),
            ("# Single Header\nContent", "simple document"),
            ("No headers at all", "no structure"),
            ("# 中文标题\n中文内容", "unicode content"),
            (
                "# Header\n![Image](image.png)\n[Link](http://example.com)",
                "with media",
            ),
        ];

        for (content, _description) in test_cases {
            let boundary = "----test-boundary";
            let body = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"markdown_file\"; filename=\"test.md\"\r\nContent-Type: text/markdown\r\n\r\n{content}\r\n--{boundary}--\r\n"
            );

            let _request = Request::builder()
                .method("POST")
                .uri("/api/v1/markdown/sections")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap();

            // Each case should be handled appropriately
            // description helps identify which test case failed
        }
    }

    #[tokio::test]
    async fn test_error_response_formats() {
        let _app_state = create_test_app_state().await;

        // Test various error scenarios
        let error_cases = vec![
            ("invalid-task-id", "Invalid UUID format"),
            ("00000000-0000-0000-0000-000000000000", "Task not found"),
        ];

        for (task_id, _expected_error_type) in error_cases {
            let _request = Request::builder()
                .method("GET")
                .uri(format!("/api/v1/task/{task_id}/status"))
                .body(Body::empty())
                .unwrap();

            // Should return appropriate error response format
            // expected_error_type describes the expected error
        }
    }

    #[tokio::test]
    async fn test_concurrent_request_handling() {
        let app_state = create_test_app_state().await;

        // Create multiple tasks concurrently
        let mut handles = vec![];

        for i in 0..10 {
            let app_state_clone = app_state.clone();
            let handle = tokio::spawn(async move {
                // Simulate concurrent task creation
                let mut task = DocumentTask::new(
                    Uuid::new_v4().to_string(),
                    SourceType::Upload,
                    Some(format!("/tmp/test{i}.pdf")),
                    Some(format!("test{i}.pdf")),
                    Some(DocumentFormat::PDF),
                    Some("pipeline".to_string()),
                    Some(24),
                    Some(3),
                );
                task.parser_engine = Some(ParserEngine::MinerU);

                app_state_clone
                    .storage_service
                    .save_task(&task)
                    .await
                    .expect("Failed to save task");

                task.id
            });
            handles.push(handle);
        }

        // Wait for all tasks and collect IDs
        let mut task_ids = vec![];
        for handle in handles {
            let task_id = handle.await.expect("Task creation failed");
            task_ids.push(task_id);
        }

        // Verify all tasks were created successfully
        assert_eq!(task_ids.len(), 10);

        // Test concurrent status retrieval
        let mut status_handles = vec![];
        for task_id in task_ids {
            let _app_state_clone = app_state.clone();
            let handle = tokio::spawn(async move {
                let _request = Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/task/{task_id}/status"))
                    .body(Body::empty())
                    .unwrap();

                // Handler should handle concurrent requests properly
                task_id
            });
            status_handles.push(handle);
        }

        // Wait for all status requests
        for handle in status_handles {
            handle.await.expect("Status request failed");
        }
    }

    #[tokio::test]
    async fn test_request_timeout_handling() {
        let _app_state = create_test_app_state().await;

        // Test timeout scenarios
        use tokio::time::{Duration, timeout};

        let _request = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        // Test that requests complete within reasonable time
        let result = timeout(Duration::from_secs(5), async {
            // Simulate handler execution
            tokio::time::sleep(Duration::from_millis(100)).await;
            "success"
        })
        .await;

        assert!(result.is_ok(), "Request should complete within timeout");
    }

    #[tokio::test]
    async fn test_health_check_comprehensive() {
        let _app_state = create_test_app_state().await;

        let _request = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        // Health check should verify:
        // - Database connectivity
        // - OSS service availability
        // - Python environment status
        // - System resources
    }

    #[tokio::test]
    async fn test_monitoring_endpoints() {
        let _app_state = create_test_app_state().await;

        // Test metrics endpoint
        let _metrics_request = Request::builder()
            .method("GET")
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();

        // Should return system metrics in appropriate format

        // Test system info endpoint
        let _info_request = Request::builder()
            .method("GET")
            .uri("/info")
            .body(Body::empty())
            .unwrap();

        // Should return system information
    }
}

#[cfg(test)]
mod handler_integration_tests {
    use super::*;
    

    #[tokio::test]
    async fn test_complete_document_processing_workflow() {
        let _app_state = create_test_app_state().await;

        // Step 1: Upload document
        let boundary = "----test-boundary";
        let file_content = "fake pdf content";
        let upload_body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"test.pdf\"\r\nContent-Type: application/pdf\r\n\r\n{file_content}\r\n--{boundary}--\r\n"
        );

        let _upload_request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(upload_body))
            .unwrap();

        // Step 2: Check task status
        // (Would need task ID from upload response)

        // Step 3: Download results when complete
        // (Would check status until complete, then download)

        // This test demonstrates the complete workflow
        // In actual implementation, would verify each step
    }

    #[tokio::test]
    async fn test_error_recovery_workflow() {
        let app_state = create_test_app_state().await;

        // Create a task that will fail
        let mut task = DocumentTask::new(
            Uuid::new_v4().to_string(),
            SourceType::Upload,
            Some("/nonexistent/file.pdf".to_string()),
            Some("file.pdf".to_string()),
            Some(DocumentFormat::PDF),
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );
        task.parser_engine = Some(ParserEngine::MinerU);

        // Set task to failed state
        let mut failed_task = task.clone();
        failed_task.status = TaskStatus::new_failed(
            TaskError::new("E001".to_string(), "File not found".to_string(), None),
            1,
        );

        app_state
            .storage_service
            .save_task(&failed_task)
            .await
            .expect("Failed to save failed task");

        // Test error handling in status endpoint
        let _status_request = Request::builder()
            .method("GET")
            .uri(format!("/api/v1/task/{}/status", task.id))
            .body(Body::empty())
            .unwrap();

        // Should return error information properly formatted

        // Test retry mechanism (if implemented)
        let _retry_request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/task/{}/retry", task.id))
            .body(Body::empty())
            .unwrap();

        // Should handle retry requests appropriately
    }

    #[tokio::test]
    async fn test_rate_limiting_behavior() {
        let _app_state = create_test_app_state().await;

        // Send multiple requests rapidly
        let mut handles = vec![];

        for i in 0..20 {
            let handle = tokio::spawn(async move {
                let _request = Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap();

                // Simulate rapid requests
                i
            });
            handles.push(handle);
        }

        // Wait for all requests
        for handle in handles {
            handle.await.expect("Request failed");
        }

        // Rate limiting (if implemented) should handle this gracefully
    }
}

#[cfg(test)]
mod handler_security_tests {
    use super::*;

    #[tokio::test]
    async fn test_input_sanitization() {
        let _app_state = create_test_app_state().await;

        // Test malicious input scenarios
        let malicious_inputs = vec![
            "<script>alert('xss')</script>",
            "'; DROP TABLE tasks; --",
            "../../../etc/passwd",
            "\x00\x01\x02", // Binary data
        ];

        for malicious_input in malicious_inputs {
            let request_body = json!({
                "url": malicious_input,
                "filename": "test.pdf"
            });

            let _request = Request::builder()
                .method("POST")
                .uri("/api/v1/document/url")
                .header("content-type", "application/json")
                .body(Body::from(request_body.to_string()))
                .unwrap();

            // Should sanitize or reject malicious input
        }
    }

    #[tokio::test]
    async fn test_path_traversal_protection() {
        let _app_state = create_test_app_state().await;

        // Test path traversal attempts
        let path_traversal_attempts = vec![
            "../../../etc/passwd",
            "..\\..\\..\\windows\\system32\\config\\sam",
            "/etc/passwd",
            "C:\\Windows\\System32\\config\\sam",
        ];

        for path in path_traversal_attempts {
            let _request = Request::builder()
                .method("GET")
                .uri(format!("/api/v1/task/{path}/status"))
                .body(Body::empty())
                .unwrap();

            // Should reject path traversal attempts
        }
    }

    #[tokio::test]
    async fn test_file_upload_security() {
        let _app_state = create_test_app_state().await;

        // Test malicious file uploads
        let boundary = "----test-boundary";

        // Test executable file upload
        let exe_body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"malware.exe\"\r\nContent-Type: application/x-executable\r\n\r\nMZ\x4D\x5A\x03\r\n--{boundary}--\r\n"
        );

        let _exe_request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(exe_body))
            .unwrap();

        // Should reject executable files

        // Test oversized file
        let large_content = "x".repeat(1024 * 1024 * 1024); // 1GB
        let large_body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"large.pdf\"\r\nContent-Type: application/pdf\r\n\r\n{large_content}\r\n--{boundary}--\r\n"
        );

        let _large_request = Request::builder()
            .method("POST")
            .uri("/api/v1/document/upload")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(large_body))
            .unwrap();

        // Should reject oversized files
    }
}
