//! 端到端集成测试
//! 
//! 使用集成测试框架进行完整的端到端测试
//! 包括文档上传、处理、存储和检索的完整流程

mod integration_test_framework;

use std::time::Duration;
use axum::http::StatusCode;
use serde_json::json;
use tower::ServiceExt;
use axum::body::Body;
use axum::http::Request;
use integration_test_framework::*;
use document_parser::models::*;

/// 端到端文档处理测试
#[tokio::test]
async fn test_end_to_end_document_processing() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    // 设置模拟服务
    env.setup_oss_mocks().await;
    env.setup_parser_mocks().await
        .expect("Failed to setup parser mocks");
    
    let app = env.create_test_app();
    
    // 创建测试PDF文件
    let pdf_content = IntegrationTestTools::generate_test_pdf_content();
    let _test_file_path = env.create_test_file("test_document.pdf", &pdf_content).await
        .expect("Failed to create test file");
    
    // 1. 上传文档
    let upload_request = Request::builder()
        .method("POST")
        .uri("/api/v1/documents/upload")
        .header("content-type", "multipart/form-data")
        .body(Body::from(pdf_content))
        .expect("Failed to build upload request");
    
    let response = app.clone().oneshot(upload_request).await
        .expect("Failed to send upload request");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    let response_body = axum::body::to_bytes(response.into_body(), usize::MAX).await
        .expect("Failed to read response body");
    let upload_response: serde_json::Value = serde_json::from_slice(&response_body)
        .expect("Failed to parse upload response");
    
    let task_id = upload_response["task_id"].as_str()
        .expect("Task ID not found in response");
    
    // 2. 等待处理完成
    let completed_task = IntegrationTestTools::wait_for_task_completion(
        &env.app_state.task_service,
        task_id,
        Duration::from_secs(30),
    ).await.expect("Task did not complete in time");
    
    // 3. 验证任务结果
    match &completed_task.status {
        TaskStatus::Completed { result_summary, .. } => {
            let summary = result_summary.as_ref().expect("Result summary not found");
            assert!(!summary.is_empty());
            assert!(summary.contains("Mock Document") || summary.contains("completed"));
        }
        _ => panic!("Task should be completed"),
    }
    
    // 4. 获取处理结果
    let get_request = Request::builder()
        .method("GET")
        .uri(&format!("/api/v1/tasks/{}", task_id))
        .body(Body::empty())
        .expect("Failed to build get request");
    
    let response = app.clone().oneshot(get_request).await
        .expect("Failed to send get request");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // 5. 获取文档内容
    let content_request = Request::builder()
        .method("GET")
        .uri(&format!("/api/v1/documents/{}/content", task_id))
        .body(Body::empty())
        .expect("Failed to build content request");
    
    let response = app.oneshot(content_request).await
        .expect("Failed to send content request");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    env.cleanup().await;
}

/// URL文档处理端到端测试
#[tokio::test]
async fn test_end_to_end_url_processing() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    // 设置模拟服务
    env.setup_oss_mocks().await;
    env.setup_parser_mocks().await
        .expect("Failed to setup parser mocks");
    
    // 模拟URL下载
    let mock_url = format!("{}/test-document.pdf", env.mock_server.uri());
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/test-document.pdf"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .set_body_bytes(IntegrationTestTools::generate_test_pdf_content()))
        .mount(&env.mock_server)
        .await;
    
    let app = env.create_test_app();
    
    // 提交URL处理请求
    let url_request = Request::builder()
        .method("POST")
        .uri("/api/v1/documents/url")
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "url": mock_url,
            "parser_engine": "mineru"
        }).to_string()))
        .expect("Failed to build URL request");
    
    let response = app.clone().oneshot(url_request).await
        .expect("Failed to send URL request");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    let response_body = axum::body::to_bytes(response.into_body(), usize::MAX).await
        .expect("Failed to read response body");
    let url_response: serde_json::Value = serde_json::from_slice(&response_body)
        .expect("Failed to parse URL response");
    
    let task_id = url_response["task_id"].as_str()
        .expect("Task ID not found in response");
    
    // 等待处理完成
    let completed_task = IntegrationTestTools::wait_for_task_completion(
        &env.app_state.task_service,
        task_id,
        Duration::from_secs(30),
    ).await.expect("URL task did not complete in time");
    
    // 验证结果
    assert!(matches!(completed_task.status, TaskStatus::Completed { .. }));
    assert_eq!(completed_task.source_type, SourceType::Url);
    
    env.cleanup().await;
}

/// 并发处理测试
#[tokio::test]
async fn test_concurrent_document_processing() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    env.setup_oss_mocks().await;
    env.setup_parser_mocks().await
        .expect("Failed to setup parser mocks");
    
    let app = env.create_test_app();
    
    // 创建多个并发任务
    let tasks: Vec<Box<dyn FnOnce() -> _ + Send>> = (0..5)
        .map(|i| {
            let app = app.clone();
            let pdf_content = IntegrationTestTools::generate_test_pdf_content();
            
            Box::new(move || async move {
                let upload_request = Request::builder()
                    .method("POST")
                    .uri("/api/v1/documents/upload")
                    .header("content-type", "multipart/form-data")
                    .body(Body::from(pdf_content))
                    .expect("Failed to build upload request");
                
                let response = app.oneshot(upload_request).await
                    .expect("Failed to send upload request");
                
                if response.status() == StatusCode::OK {
                    Ok(i)
                } else {
                    Err(anyhow::anyhow!("Upload failed with status: {}", response.status()))
                }
            }) as Box<dyn FnOnce() -> _ + Send>
        })
        .collect();
    
    let results = ConcurrencyTestTools::run_concurrent_tasks(tasks, 3).await;
    
    // 验证所有任务都成功
    assert_eq!(results.len(), 5);
    for result in results {
        assert!(result.is_ok(), "Concurrent task failed: {:?}", result);
    }
    
    env.cleanup().await;
}

/// 错误处理端到端测试
#[tokio::test]
async fn test_end_to_end_error_handling() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    let app = env.create_test_app();
    
    // 测试无效文件格式
    let invalid_content = b"This is not a valid document";
    let upload_request = Request::builder()
        .method("POST")
        .uri("/api/v1/documents/upload")
        .header("content-type", "multipart/form-data")
        .body(Body::from(invalid_content.to_vec()))
        .expect("Failed to build upload request");
    
    let response = app.clone().oneshot(upload_request).await
        .expect("Failed to send upload request");
    
    // 应该返回错误状态
    assert!(response.status().is_client_error() || response.status().is_server_error());
    
    // 测试无效URL
    let invalid_url_request = Request::builder()
        .method("POST")
        .uri("/api/v1/documents/url")
        .header("content-type", "application/json")
        .body(Body::from(json!({
            "url": "invalid-url",
            "parser_engine": "mineru"
        }).to_string()))
        .expect("Failed to build invalid URL request");
    
    let response = app.clone().oneshot(invalid_url_request).await
        .expect("Failed to send invalid URL request");
    
    assert!(response.status().is_client_error());
    
    // 测试获取不存在的任务
    let nonexistent_request = Request::builder()
        .method("GET")
        .uri("/api/v1/tasks/nonexistent-task-id")
        .body(Body::empty())
        .expect("Failed to build nonexistent request");
    
    let response = app.oneshot(nonexistent_request).await
        .expect("Failed to send nonexistent request");
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    
    env.cleanup().await;
}

/// 性能基准测试
#[tokio::test]
async fn test_performance_benchmark() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    env.setup_oss_mocks().await;
    env.setup_parser_mocks().await
        .expect("Failed to setup parser mocks");
    
    let mut benchmark = PerformanceBenchmark::new();
    let app = env.create_test_app();
    
    benchmark.checkpoint("Environment setup");
    
    // 创建测试文件
    let pdf_content = IntegrationTestTools::generate_test_pdf_content();
    benchmark.checkpoint("Test file creation");
    
    // 上传文档
    let upload_request = Request::builder()
        .method("POST")
        .uri("/api/v1/documents/upload")
        .header("content-type", "multipart/form-data")
        .body(Body::from(pdf_content))
        .expect("Failed to build upload request");
    
    let response = app.clone().oneshot(upload_request).await
        .expect("Failed to send upload request");
    
    benchmark.checkpoint("Document upload");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    let response_body = axum::body::to_bytes(response.into_body(), usize::MAX).await
        .expect("Failed to read response body");
    let upload_response: serde_json::Value = serde_json::from_slice(&response_body)
        .expect("Failed to parse upload response");
    
    let task_id = upload_response["task_id"].as_str()
        .expect("Task ID not found in response");
    
    benchmark.checkpoint("Response parsing");
    
    // 等待处理完成
    let _completed_task = IntegrationTestTools::wait_for_task_completion(
        &env.app_state.task_service,
        task_id,
        Duration::from_secs(30),
    ).await.expect("Task did not complete in time");
    
    benchmark.checkpoint("Document processing");
    
    // 获取结果
    let get_request = Request::builder()
        .method("GET")
        .uri(&format!("/api/v1/tasks/{}", task_id))
        .body(Body::empty())
        .expect("Failed to build get request");
    
    let response = app.oneshot(get_request).await
        .expect("Failed to send get request");
    
    benchmark.checkpoint("Result retrieval");
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // 输出性能报告
    println!("{}", benchmark.report());
    
    env.cleanup().await;
}

/// 数据一致性测试
#[tokio::test]
async fn test_data_consistency() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    env.setup_oss_mocks().await;
    env.setup_parser_mocks().await
        .expect("Failed to setup parser mocks");
    
    let task_service = &env.app_state.task_service;
    
    // 创建任务
    let task = task_service.create_task(
        SourceType::Upload,
        Some("/tmp/test.pdf".to_string()),
        DocumentFormat::PDF,
    ).await.expect("Failed to create task");
    
    let task_id = task.id.clone();
    
    // 验证任务状态一致性
    let retrieved_task = task_service.get_task(&task_id).await
        .expect("Failed to get task")
        .expect("Task not found");
    
    assert_eq!(task.id, retrieved_task.id);
    assert_eq!(task.source_type, retrieved_task.source_type);
    assert_eq!(task.document_format, retrieved_task.document_format);
    
    // 更新任务状态
    let new_status = TaskStatus::Processing {
        stage: ProcessingStage::FormatDetection,
        progress_details: Some(ProgressDetails::new("Testing consistency".to_string())),
        started_at: chrono::Utc::now(),
    };
    
    task_service.update_task_status(&task_id, new_status.clone()).await
        .expect("Failed to update task status");
    
    // 验证状态更新一致性
    let updated_task = task_service.get_task(&task_id).await
        .expect("Failed to get updated task")
        .expect("Updated task not found");
    
    assert!(matches!(updated_task.status, TaskStatus::Processing { .. }));
    
    // 验证状态转换有效性
    assert!(IntegrationTestTools::validate_task_status_transition(
        &task.status,
        &updated_task.status
    ));
    
    env.cleanup().await;
}

/// 资源清理测试
#[tokio::test]
async fn test_resource_cleanup() {
    let env = IntegrationTestEnvironment::new().await
        .expect("Failed to create test environment");
    
    let task_service = &env.app_state.task_service;
    
    // 创建多个任务
    let mut task_ids = Vec::new();
    for i in 0..5 {
        let task = task_service.create_task(
            SourceType::Upload,
            Some(format!("/tmp/test_{}.pdf", i)),
            DocumentFormat::PDF,
        ).await.expect("Failed to create task");
        task_ids.push(task.id);
    }
    
    // 验证任务存在
    for task_id in &task_ids {
        let task = task_service.get_task(task_id).await
            .expect("Failed to get task");
        assert!(task.is_some());
    }
    
    // 执行清理
    env.cleanup().await;
    
    // 注意：在实际实现中，这里可能需要等待清理完成
    // 或者验证特定的清理行为
    
    println!("Resource cleanup test completed");
}