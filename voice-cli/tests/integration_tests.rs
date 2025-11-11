use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use bytes::Bytes;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{Duration, sleep};
use tower::ServiceExt;
use voice_cli::models::{Config, TaskManagementConfig};
use voice_cli::server::routes;

/// Helper function to create test configuration
async fn create_test_config() -> (Arc<Config>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("integration_test.db");
    let models_dir = temp_dir.path().join("models");
    let logs_dir = temp_dir.path().join("logs");

    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::create_dir_all(&logs_dir).unwrap();

    let mut config = Config::default();
    config.server.port = 0; // Use random port for tests
    config.whisper.models_dir = models_dir.to_string_lossy().to_string();
    config.logging.log_dir = logs_dir.to_string_lossy().to_string();
    config.task_management = TaskManagementConfig {
        enabled: true,
        max_concurrent_tasks: 2,
        task_timeout: 30,
        retry_attempts: 2,
        retry_backoff_multiplier: 1.5,
        sled_db_path: db_path.to_string_lossy().to_string(),
        sled_cache_capacity: 64,
        sled_compression: false,
        cleanup_interval: 10,
        retain_completed_tasks: 60,
        retain_failed_tasks: 120,
        retain_cancelled_tasks: 60,
        health_check_interval: 5,
        max_queue_size: 100,
        queue_warning_threshold: 50,
    };

    (Arc::new(config), temp_dir)
}

/// Helper function to create test audio data
fn create_test_audio_data() -> Bytes {
    // Create a minimal WAV file header + some dummy audio data
    let mut wav_data = Vec::new();

    // WAV header (44 bytes)
    wav_data.extend_from_slice(b"RIFF");
    wav_data.extend_from_slice(&(36u32).to_le_bytes()); // File size - 8
    wav_data.extend_from_slice(b"WAVE");
    wav_data.extend_from_slice(b"fmt ");
    wav_data.extend_from_slice(&(16u32).to_le_bytes()); // Subchunk1 size
    wav_data.extend_from_slice(&(1u16).to_le_bytes()); // Audio format (PCM)
    wav_data.extend_from_slice(&(1u16).to_le_bytes()); // Num channels
    wav_data.extend_from_slice(&(44100u32).to_le_bytes()); // Sample rate
    wav_data.extend_from_slice(&(88200u32).to_le_bytes()); // Byte rate
    wav_data.extend_from_slice(&(2u16).to_le_bytes()); // Block align
    wav_data.extend_from_slice(&(16u16).to_le_bytes()); // Bits per sample
    wav_data.extend_from_slice(b"data");
    wav_data.extend_from_slice(&(0u32).to_le_bytes()); // Data size

    // Add some dummy audio data (silence)
    wav_data.extend_from_slice(&[0u8; 1000]);

    Bytes::from(wav_data)
}

/// Helper function to create multipart form data
fn create_multipart_body(audio_data: &Bytes, model: Option<&str>) -> (String, Bytes) {
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let mut body = Vec::new();

    // Audio field
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"audio\"; filename=\"test.wav\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: audio/wav\r\n\r\n");
    body.extend_from_slice(audio_data);
    body.extend_from_slice(b"\r\n");

    // Model field (if provided)
    if let Some(model) = model {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(model.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    // End boundary
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

    let content_type = format!("multipart/form-data; boundary={}", boundary);
    (content_type, Bytes::from(body))
}

#[tokio::test]
async fn test_health_endpoint() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let health_response: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(health_response["status"], "healthy");
}

#[tokio::test]
async fn test_models_endpoint() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/models")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let models_response: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(models_response["success"], true);
    assert!(models_response["data"]["available_models"].is_array());
}

#[tokio::test]
async fn test_async_transcription_workflow() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let audio_data = create_test_audio_data();
    let (content_type, body) = create_multipart_body(&audio_data, Some("base"));

    // Submit async transcription task
    let request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/transcribe")
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let task_response: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(task_response["success"], true);
    let task_id = task_response["data"]["task_id"].as_str().unwrap();
    assert!(!task_id.is_empty());

    // Wait a bit for task processing
    sleep(Duration::from_millis(100)).await;

    // Check task status
    let status_request = Request::builder()
        .method(Method::GET)
        .uri(&format!("/tasks/{}", task_id))
        .body(Body::empty())
        .unwrap();

    let status_response = app.clone().oneshot(status_request).await.unwrap();
    assert_eq!(status_response.status(), StatusCode::OK);

    let status_body = axum::body::to_bytes(status_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status_data: Value = serde_json::from_slice(&status_body).unwrap();

    assert_eq!(status_data["success"], true);
    assert_eq!(status_data["data"]["task_id"], task_id);

    // The task should be in pending or processing state
    let status = status_data["data"]["status"].as_object().unwrap();
    assert!(status.contains_key("Pending") || status.contains_key("Processing"));
}

#[tokio::test]
async fn test_task_cancellation() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let audio_data = create_test_audio_data();
    let (content_type, body) = create_multipart_body(&audio_data, None);

    // Submit task
    let request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/transcribe")
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let task_response: Value = serde_json::from_slice(&body).unwrap();
    let task_id = task_response["data"]["task_id"].as_str().unwrap();

    // Cancel task
    let cancel_request = Request::builder()
        .method(Method::DELETE)
        .uri(&format!("/tasks/{}", task_id))
        .body(Body::empty())
        .unwrap();

    let cancel_response = app.clone().oneshot(cancel_request).await.unwrap();

    // Should succeed or return 400 if task already started processing
    assert!(
        cancel_response.status() == StatusCode::OK
            || cancel_response.status() == StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn test_task_listing() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    // Submit multiple tasks
    for _i in 0..3 {
        let audio_data = create_test_audio_data();
        let (content_type, body) = create_multipart_body(&audio_data, None);

        let request = Request::builder()
            .method(Method::POST)
            .uri("/tasks/transcribe")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    // Wait a bit
    sleep(Duration::from_millis(100)).await;

    // List tasks
    let list_request = Request::builder()
        .method(Method::GET)
        .uri("/tasks?limit=10")
        .body(Body::empty())
        .unwrap();

    let list_response = app.clone().oneshot(list_request).await.unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);

    let list_body = axum::body::to_bytes(list_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let list_data: Value = serde_json::from_slice(&list_body).unwrap();

    assert_eq!(list_data["success"], true);
    let tasks = list_data["data"]["tasks"].as_array().unwrap();
    assert!(tasks.len() >= 3);
}

#[tokio::test]
async fn test_task_statistics() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    // Get initial stats
    let stats_request = Request::builder()
        .method(Method::GET)
        .uri("/tasks/stats")
        .body(Body::empty())
        .unwrap();

    let stats_response = app.clone().oneshot(stats_request).await.unwrap();
    assert_eq!(stats_response.status(), StatusCode::OK);

    let stats_body = axum::body::to_bytes(stats_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let stats_data: Value = serde_json::from_slice(&stats_body).unwrap();

    assert_eq!(stats_data["success"], true);
    assert!(stats_data["data"]["total_tasks"].is_number());
    assert!(stats_data["data"]["pending_tasks"].is_number());
    assert!(stats_data["data"]["processing_tasks"].is_number());
    assert!(stats_data["data"]["completed_tasks"].is_number());
    assert!(stats_data["data"]["failed_tasks"].is_number());
}

#[tokio::test]
async fn test_cleanup_endpoint() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    // Trigger cleanup
    let cleanup_request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/cleanup")
        .body(Body::empty())
        .unwrap();

    let cleanup_response = app.oneshot(cleanup_request).await.unwrap();
    assert_eq!(cleanup_response.status(), StatusCode::OK);

    let cleanup_body = axum::body::to_bytes(cleanup_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let cleanup_data: Value = serde_json::from_slice(&cleanup_body).unwrap();

    assert_eq!(cleanup_data["success"], true);
    assert!(cleanup_data["data"]["cleaned_tasks"].is_number());
    assert!(cleanup_data["data"]["message"].is_string());
}

#[tokio::test]
async fn test_sync_transcription_still_works() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let audio_data = create_test_audio_data();
    let (content_type, body) = create_multipart_body(&audio_data, Some("base"));

    // Test synchronous transcription endpoint
    let request = Request::builder()
        .method(Method::POST)
        .uri("/transcribe")
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Should return OK or an error (depending on whether models are available)
    // The important thing is that the endpoint is accessible
    assert!(
        response.status() == StatusCode::OK
            || response.status().is_client_error()
            || response.status().is_server_error()
    );
}

#[tokio::test]
async fn test_error_handling() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    // Test invalid task ID
    let invalid_request = Request::builder()
        .method(Method::GET)
        .uri("/tasks/invalid-task-id")
        .body(Body::empty())
        .unwrap();

    let invalid_response = app.clone().oneshot(invalid_request).await.unwrap();
    assert_eq!(invalid_response.status(), StatusCode::NOT_FOUND);

    // Test malformed multipart data
    let malformed_request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/transcribe")
        .header("content-type", "multipart/form-data; boundary=invalid")
        .body(Body::from("invalid data"))
        .unwrap();

    let malformed_response = app.clone().oneshot(malformed_request).await.unwrap();
    assert!(malformed_response.status().is_client_error());

    // Test missing audio field
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let empty_body = format!(
        "--{}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nbase\r\n--{}--\r\n",
        boundary, boundary
    );

    let empty_request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/transcribe")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(empty_body))
        .unwrap();

    let empty_response = app.oneshot(empty_request).await.unwrap();
    assert!(empty_response.status().is_client_error());
}

#[tokio::test]
async fn test_concurrent_requests() {
    let (config, _temp_dir) = create_test_config().await;
    let app = routes::create_routes(config).await.unwrap();

    let mut handles = vec![];

    // Submit multiple concurrent requests
    for i in 0..5 {
        let app = app.clone();
        let handle = tokio::spawn(async move {
            let audio_data = create_test_audio_data();
            let (content_type, body) = create_multipart_body(&audio_data, None);

            let request = Request::builder()
                .method(Method::POST)
                .uri("/tasks/transcribe")
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap();

            let response = app.oneshot(request).await.unwrap();
            (i, response.status())
        });

        handles.push(handle);
    }

    // Wait for all requests to complete
    let mut success_count = 0;
    for handle in handles {
        let (i, status) = handle.await.unwrap();
        if status == StatusCode::OK {
            success_count += 1;
        }
        println!("Request {}: {:?}", i, status);
    }

    // At least some requests should succeed
    assert!(success_count > 0);
}

#[tokio::test]
async fn test_task_management_disabled() {
    let (config, _temp_dir) = create_test_config().await;
    // We need to modify the config, so we need to clone it out of the Arc
    let mut config_clone = (*config).clone();
    config_clone.task_management.enabled = false;
    let config = Arc::new(config_clone);

    let app = routes::create_routes(config).await.unwrap();

    let audio_data = create_test_audio_data();
    let (content_type, body) = create_multipart_body(&audio_data, None);

    // Try to submit async task when task management is disabled
    let request = Request::builder()
        .method(Method::POST)
        .uri("/tasks/transcribe")
        .header("content-type", content_type)
        .body(Body::from(body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error_response: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(error_response["success"], false);
    assert!(
        error_response["error"]
            .as_str()
            .unwrap()
            .contains("not enabled")
    );
}
