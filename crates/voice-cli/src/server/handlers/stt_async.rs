//! 异步转录端点：文件上传与 URL 两种提交方式，入队即返回任务 ID。

use super::*;

/// 提交异步转录任务
/// POST /tasks/transcribe
#[utoipa::path(
    post,
    path = "/api/v1/tasks/transcribe",
    tag = "异步转录",
    summary = "提交音频转录任务",
    description = "上传音频文件进行异步转录处理，立即返回任务ID用于跟踪进度",
    request_body(
        content = String,
        description = "multipart/form-data 包含音频文件和可选参数",
        content_type = "multipart/form-data"
    ),
    responses(
        (status = 200, description = "任务提交成功", body = HttpResult<AsyncTaskResponse>),
        (status = 400, description = "请求无效", body = String),
        (status = 413, description = "文件过大", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn async_transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<HttpResult<AsyncTaskResponse>, VoiceCliError> {
    let task_id = crate::utils::generate_task_id();
    info!(
        "Start processing asynchronous transcription request: {}",
        task_id
    );

    // 使用流式处理避免内存占用
    let (audio_file_path, request) = extract_transcription_request_streaming(
        multipart,
        &task_id,
        &state.audio_file_manager.storage_dir,
    )
    .await?;

    // 元数据提取移到worker中执行，避免阻塞接口响应

    // 提交任务到队列 - 使用无锁管理器
    info!("Start submitting tasks to the queue...");
    let mut storage = state.apalis_storage.clone();
    let manager = state.lock_free_apalis_manager.as_ref();

    // 如果请求中没有指定模型，使用配置中的默认模型
    let model = request
        .model
        .or_else(|| Some(state.config.whisper.default_model.clone()));

    info!("Submit tasks using lock-free ApalisManager...");
    let result = manager
        .submit_task(
            &mut storage,
            audio_file_path,
            request.filename,
            model,
            request.response_format,
            request
                .language
                .or_else(|| state.config.whisper.engine.default_language.clone()),
            request
                .initial_prompt
                .or_else(|| state.config.whisper.engine.default_initial_prompt.clone()),
        )
        .await;
    info!(
        "The task submission operation is completed, result: {:?}",
        result
    );
    let returned_task_id = result?;

    info!(
        "Asynchronous transcription task submitted successfully: {}",
        returned_task_id
    );

    let response = AsyncTaskResponse {
        task_id: returned_task_id,
        status: TaskStatus::Pending {
            queued_at: chrono::Utc::now(),
        },
        estimated_completion: None,
    };

    Ok(HttpResult::success(response))
}

/// 通过URL提交异步转录任务
/// POST /transcribeFromUrl
#[utoipa::path(
    post,
    path = "/api/v1/tasks/transcribeFromUrl",
    tag = "异步转录",
    summary = "通过URL提交音频转录任务",
    description = "通过URL下载音频文件进行异步转录处理，立即返回任务ID用于跟踪进度",
    request_body(
        content = UrlTranscriptionRequest,
        description = "URL transcription request data",
        content_type = "application/json"
    ),
    responses(
        (status = 200, description = "任务提交成功", body = HttpResult<AsyncTaskResponse>),
        (status = 400, description = "请求无效", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn transcribe_from_url_handler(
    State(state): State<AppState>,
    Json(request): Json<UrlTranscriptionRequest>,
) -> Result<HttpResult<AsyncTaskResponse>, VoiceCliError> {
    let task_id = crate::utils::generate_task_id();
    info!(
        "Start processing asynchronous transcription request of URL: {} - URL: {}",
        task_id, request.url
    );

    // 从URL中提取文件名
    let filename =
        extract_filename_from_url(&request.url).unwrap_or_else(|| "audio_from_url".to_string());

    // 提交URL任务到队列 - 使用无锁管理器
    info!("Start submitting URL tasks to the queue...");
    let mut storage = state.apalis_storage.clone();
    let manager = state.lock_free_apalis_manager.as_ref();

    // 如果请求中没有指定模型，使用配置中的默认模型
    let model = request
        .model
        .or_else(|| Some(state.config.whisper.default_model.clone()));

    info!("Submit URL tasks using lock-free ApalisManager...");
    let result = manager
        .submit_task_for_url(
            &mut storage,
            request.url,
            filename,
            model,
            request.response_format,
            request
                .language
                .or_else(|| state.config.whisper.engine.default_language.clone()),
            request
                .initial_prompt
                .or_else(|| state.config.whisper.engine.default_initial_prompt.clone()),
        )
        .await;
    info!(
        "URL task submission operation completed, result: {:?}",
        result
    );
    let returned_task_id = result?;

    info!(
        "URL asynchronous transcription task submitted successfully: {}",
        returned_task_id
    );

    let response = AsyncTaskResponse {
        task_id: returned_task_id,
        status: TaskStatus::Pending {
            queued_at: chrono::Utc::now(),
        },
        estimated_completion: None,
    };

    Ok(HttpResult::success(response))
}
