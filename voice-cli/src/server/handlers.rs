use crate::VoiceCliError;
use crate::models::{
    AsyncTaskResponse, CancelResponse, Config, HealthResponse, HttpResult, ModelsResponse,
    RetryResponse, SimpleTaskStatus, TaskStatsResponse, TaskStatus, TaskStatusResponse, TranscriptionResponse,
};
use crate::services::{LockFreeApalisManager, AudioFileManager, ModelService, TranscriptionTask};
use apalis_sql::sqlite::SqliteStorage;
use axum::extract::{Multipart, State};
use bytes::Bytes;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use utoipa;

#[derive(Clone, Debug)]
pub struct AppState {
    pub config: Arc<Config>,
    pub model_service: Arc<ModelService>,
    pub lock_free_apalis_manager: Arc<LockFreeApalisManager>,
    pub apalis_storage: SqliteStorage<TranscriptionTask>,
    pub audio_file_manager: Arc<AudioFileManager>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // 初始化无锁 Apalis 管理器
        info!("初始化无锁 Apalis 任务管理器");
        let (manager, storage) =
            LockFreeApalisManager::new(config.task_management.clone(), model_service.clone()).await?;

        // 启动 worker
        manager
            .start_worker(storage.clone(), model_service.clone())
            .await?;

        let lock_free_apalis_manager = Arc::new(manager);
        let apalis_storage = storage;

        // 初始化音频文件管理器
        let audio_file_manager = Arc::new(
            AudioFileManager::new("./data/audio")
                .map_err(|e| VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))?,
        );

        Ok(Self {
            config,
            model_service,
            lock_free_apalis_manager,
            apalis_storage,
            audio_file_manager,
            start_time: SystemTime::now(),
        })
    }

    /// 优雅关闭
    pub async fn shutdown(&self) {
        info!("关闭应用状态");
        
        // 优雅关闭 Apalis 管理器
        if let Err(e) = self.lock_free_apalis_manager.shutdown().await {
            warn!("关闭 Apalis 管理器失败: {}", e);
        }
        
        info!("应用状态关闭完成");
    }
}

/// 健康检查端点
/// GET /health
#[utoipa::path(
    get,
    path = "/health",
    tag = "健康检查",
    summary = "健康检查",
    description = "检查服务是否正常运行",
    responses(
        (status = 200, description = "服务正常", body = HealthResponse),
        (status = 500, description = "服务异常", body = String)
    ),
)]
pub async fn health_handler(State(state): State<AppState>) -> HttpResult<HealthResponse> {
    let uptime = SystemTime::now()
        .duration_since(state.start_time)
        .unwrap_or_default();

    HttpResult::success(HealthResponse {
        status: "healthy".to_string(),
        models_loaded: vec![],
        uptime: uptime.as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// 获取模型列表
/// GET /models
#[utoipa::path(
    get,
    path = "/models",
    tag = "模型管理",
    summary = "获取可用模型列表",
    description = "获取当前支持的语音转录模型列表",
    responses(
        (status = 200, description = "模型列表", body = ModelsResponse),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn models_list_handler(State(_state): State<AppState>) -> HttpResult<ModelsResponse> {
    // 简化版本，返回空模型列表
    HttpResult::success(ModelsResponse {
        available_models: vec!["base".to_string(), "small".to_string()],
        loaded_models: vec!["base".to_string()],
        model_info: std::collections::HashMap::new(),
    })
}

/// 同步转录处理
/// POST /transcribe
#[utoipa::path(
    post,
    path = "/transcribe",
    tag = "转录",
    summary = "同步音频转录",
    description = "上传音频文件进行同步转录处理，立即返回结果",
    request_body(
        content = String,
        description = "multipart/form-data 包含音频文件和可选参数",
        content_type = "multipart/form-data"
    ),
    responses(
        (status = 200, description = "转录成功", body = TranscriptionResponse),
        (status = 400, description = "请求无效", body = String),
        (status = 413, description = "文件过大", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<HttpResult<TranscriptionResponse>, VoiceCliError> {
    // 解析 multipart 请求
    let (audio_data, request) = extract_transcription_request(multipart).await?;

    // 验证音频文件
    validate_audio_file(&audio_data, &request.filename)?;

    // 使用转录引擎处理
    let transcription_engine = crate::services::TranscriptionEngine::new(state.model_service);

    // 先保存临时文件，然后转录
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join(&request.filename);

    tokio::fs::write(&temp_file, &audio_data)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("写入临时文件失败: {}", e)))?;

    let result = transcription_engine
        .transcribe_compatible_audio(
            "base", // 默认模型
            &temp_file, 3600, // timeout_secs
        )
        .await?;

    // 清理临时文件
    let _ = tokio::fs::remove_file(&temp_file).await;

    // 转换 TranscriptionResult 到 TranscriptionResponse
    let response = TranscriptionResponse {
        text: result.text,
        segments: result
            .segments
            .into_iter()
            .map(|s| crate::models::Segment {
                start: s.start_time as f32 / 1000.0, // Convert from ms to seconds
                end: s.end_time as f32 / 1000.0,     // Convert from ms to seconds
                text: s.text,
                confidence: s.confidence,
            })
            .collect(),
        language: result.language,
        duration: None,       // 简化版本
        processing_time: 0.0, // 简化版本
    };

    info!("同步转录完成: {} 字符", response.text.len());
    Ok(HttpResult::success(response))
}

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
        (status = 200, description = "任务提交成功", body = AsyncTaskResponse),
        (status = 400, description = "请求无效", body = String),
        (status = 413, description = "文件过大", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn async_transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<HttpResult<AsyncTaskResponse>, VoiceCliError> {
    let task_id = generate_task_id();
    info!("开始处理异步转录请求: {}", task_id);

    // 解析 multipart 请求
    let (audio_data, request) = extract_transcription_request(multipart).await?;

    // 验证音频文件
    validate_audio_file(&audio_data, &request.filename)?;

    // 保存音频文件 - 使用共享的音频文件管理器
    let audio_file_path = state
        .audio_file_manager
        .save_audio_file(&task_id, &audio_data, &request.filename)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存音频文件失败: {}", e)))?;

    // 提交任务到队列 - 使用无锁管理器
    info!("开始提交任务到队列...");
    let mut storage = state.apalis_storage.clone();
    let manager = state.lock_free_apalis_manager.as_ref();
    
    info!("使用无锁 ApalisManager 提交任务...");
    let result = manager
        .submit_task(
            &mut storage,
            audio_file_path,
            request.filename,
            request.model,
            request.response_format,
        )
        .await;
    info!("任务提交操作完成，结果: {:?}", result);
    let returned_task_id = result?;

    info!("异步转录任务提交成功: {}", returned_task_id);

    let response = AsyncTaskResponse {
        task_id: returned_task_id,
        status: TaskStatus::Pending {
            queued_at: chrono::Utc::now(),
        },
        estimated_completion: None,
    };

    Ok(HttpResult::success(response))
}

/// 获取任务状态
/// GET /tasks/:task_id
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}",
    tag = "任务管理",
    summary = "获取任务状态",
    description = "根据任务ID查询转录任务的当前状态",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "状态获取成功", body = TaskStatusResponse),
        (status = 404, description = "任务不存在", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn get_task_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<TaskStatusResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    match manager.get_task_status(&task_id).await? {
        Some(status) => {
            info!("获取任务状态成功: {} -> {:?}", task_id, status);
            let message = match &status {
                TaskStatus::Completed { result_summary, .. } => result_summary.clone(),
                TaskStatus::Failed { error, .. } => Some(error.to_string()),
                TaskStatus::Cancelled { reason, .. } => reason.clone(),
                _ => None,
            };
            
            let response = TaskStatusResponse {
                task_id: task_id.clone(),
                status: SimpleTaskStatus::from(&status),
                message,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            Ok(HttpResult::success(response))
        }
        None => {
            warn!("任务不存在: {}", task_id);
            Err(VoiceCliError::NotFound(format!(
                "任务 '{}' 不存在",
                task_id
            )))
        }
    }
}

/// 获取任务结果
/// GET /tasks/:task_id/result
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/result",
    tag = "任务管理",
    summary = "获取转录结果",
    description = "获取已完成任务的转录结果",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "结果获取成功", body = TranscriptionResponse),
        (status = 404, description = "任务不存在或结果不可用", body = String),
        (status = 400, description = "任务未完成", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn get_task_result_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<TranscriptionResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    match manager.get_task_result(&task_id).await? {
        Some(result) => {
            info!(
                "获取任务结果成功: {} -> {} 字符",
                task_id,
                result.text.len()
            );
            Ok(HttpResult::success(result))
        }
        None => {
            warn!("任务结果不可用: {}", task_id);
            Err(VoiceCliError::NotFound(format!(
                "任务 '{}' 的结果不可用",
                task_id
            )))
        }
    }
}

/// 取消任务
/// DELETE /tasks/:task_id
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}",
    tag = "任务管理", 
    summary = "取消任务",
    description = "取消待处理或正在处理的转录任务",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "取消成功", body = CancelResponse),
        (status = 404, description = "任务不存在", body = String),
        (status = 400, description = "任务无法取消", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn cancel_task_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<CancelResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    let cancelled = manager.cancel_task(&task_id).await?;

    let response = CancelResponse {
        task_id: task_id.clone(),
        cancelled,
        message: if cancelled {
            format!("任务 {} 已取消", task_id)
        } else {
            format!("任务 {} 无法取消（可能已完成或失败）", task_id)
        },
    };

    info!("任务取消操作: {} -> {}", task_id, response.message);
    Ok(HttpResult::success(response))
}

/// 取消任务 (POST版本)
/// POST /tasks/:task_id/cancel
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/cancel",
    tag = "任务管理", 
    summary = "取消任务",
    description = "取消待处理或正在处理的转录任务（POST方式）",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "取消成功", body = CancelResponse),
        (status = 404, description = "任务不存在", body = String),
        (status = 400, description = "任务无法取消", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn cancel_task_post_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<CancelResponse>, VoiceCliError> {
    // 复用已有的取消逻辑
    cancel_task_handler(
        State(state),
        axum::extract::Path(task_id),
    )
    .await
}

/// 重试任务
/// POST /tasks/:task_id/retry
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/retry",
    tag = "任务管理",
    summary = "重试任务",
    description = "重试已失败或已取消的转录任务",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "重试成功", body = RetryResponse),
        (status = 404, description = "任务不存在", body = String),
        (status = 400, description = "任务无法重试", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn retry_task_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<RetryResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    let retried = manager.retry_task(&task_id).await?;

    let response = RetryResponse {
        task_id: task_id.clone(),
        retried,
        message: if retried {
            format!("任务 {} 已重新提交", task_id)
        } else {
            format!("任务 {} 无法重试（可能不存在或正在处理中）", task_id)
        },
    };

    info!("任务重试操作: {} -> {}", task_id, response.message);
    Ok(HttpResult::success(response))
}

/// 获取任务统计信息
/// GET /tasks/stats
#[utoipa::path(
    get,
    path = "/api/v1/tasks/stats",
    tag = "任务管理",
    summary = "获取任务统计信息",
    description = "获取当前任务执行情况的统计信息，包括各状态任务数量、平均执行时间等",
    responses(
        (status = 200, description = "统计信息获取成功", body = TaskStatsResponse),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn get_tasks_stats_handler(
    State(state): State<AppState>,
) -> Result<HttpResult<TaskStatsResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    let stats = manager.get_tasks_stats().await?;

    info!("获取任务统计信息: 总共 {} 个任务", stats.total_tasks);
    Ok(HttpResult::success(stats))
}

// ===== 辅助函数 =====

/// 转录请求数据
#[derive(Debug)]
struct TranscriptionRequest {
    filename: String,
    model: Option<String>,
    response_format: Option<String>,
}

/// 解析 multipart 请求
async fn extract_transcription_request(
    mut multipart: Multipart,
) -> Result<(Bytes, TranscriptionRequest), VoiceCliError> {
    let mut audio_data: Option<Bytes> = None;
    let mut filename: Option<String> = None;
    let mut model: Option<String> = None;
    let mut response_format: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| VoiceCliError::MultipartError(format!("解析 multipart 失败: {}", e)))?
    {
        let field_name = field.name().unwrap_or("unknown");

        match field_name {
            "audio" => {
                filename = field.file_name().map(|s| s.to_string());
                let data = field.bytes().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("读取音频数据失败: {}", e))
                })?;
                audio_data = Some(data);
                info!(
                    "接收音频文件: {} bytes, 文件名: {:?}",
                    audio_data.as_ref().unwrap().len(),
                    filename
                );
            }
            "model" => {
                model = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("解析模型参数失败: {}", e))
                })?);
            }
            "response_format" => {
                response_format = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("解析响应格式参数失败: {}", e))
                })?);
            }
            _ => {
                warn!("忽略未知字段: {}", field_name);
            }
        }
    }

    let audio_data = audio_data.ok_or_else(|| VoiceCliError::MissingField("audio".to_string()))?;

    // 生成文件名（如果没有提供）
    let filename = filename.unwrap_or_else(|| format!("audio_{}.wav", generate_task_id()));

    let request = TranscriptionRequest {
        filename,
        model,
        response_format,
    };

    Ok((audio_data, request))
}

/// 验证音频文件
fn validate_audio_file(audio_data: &Bytes, filename: &str) -> Result<(), VoiceCliError> {
    const MAX_FILE_SIZE: usize = 200 * 1024 * 1024; // 200MB

    if audio_data.len() > MAX_FILE_SIZE {
        return Err(VoiceCliError::FileTooLarge {
            size: audio_data.len(),
            max: MAX_FILE_SIZE,
        });
    }

    info!(
        "音频文件验证通过: {} ({} bytes)",
        filename,
        audio_data.len()
    );
    Ok(())
}

/// 生成任务 ID
fn generate_task_id() -> String {
    format!(
        "task_{}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
        std::process::id()
    )
}
