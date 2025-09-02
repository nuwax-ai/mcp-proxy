use crate::VoiceCliError;
use crate::models::{
    AsyncTaskResponse, CancelResponse, Config, DeleteResponse, HealthResponse, HttpResult,
    ModelsResponse, RetryResponse, SimpleTaskStatus, TaskStatsResponse, TaskStatus,
    TaskStatusResponse, TranscriptionResponse, TtsSyncRequest, TtsAsyncRequest, TtsTaskResponse,
};
use crate::services::{
    AudioFileManager, AudioFormatDetector, LockFreeApalisManager, ModelService, TranscriptionTask, TtsService,
};
use apalis_sql::sqlite::SqliteStorage;
use axum::extract::{Multipart, State, Json};
use futures::TryStreamExt;
use std::path::{Path, PathBuf};
use axum::response::IntoResponse;
use tower_http::body::Full;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use url::Url;
use utoipa;

#[derive(Clone, Debug)]
pub struct AppState {
    pub config: Arc<Config>,
    pub model_service: Arc<ModelService>,
    pub lock_free_apalis_manager: Arc<LockFreeApalisManager>,
    pub apalis_storage: SqliteStorage<TranscriptionTask>,
    pub audio_file_manager: Arc<AudioFileManager>,
    pub tts_service: Arc<TtsService>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // 初始化无锁 Apalis 管理器
        info!("初始化无锁 Apalis 任务管理器");
        let (manager, storage) =
            LockFreeApalisManager::new(config.task_management.clone(), model_service.clone())
                .await?;

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

        // 初始化TTS服务
        info!("初始化TTS服务");
        let tts_service = Arc::new(
            TtsService::new(
                config.tts.python_path.clone(),
                config.tts.model_path.clone(),
            ).map_err(|e| VoiceCliError::Config(format!("创建TTS服务失败: {}", e)))?
        );

        Ok(Self {
            config,
            model_service,
            lock_free_apalis_manager,
            apalis_storage,
            audio_file_manager,
            tts_service,
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
        (status = 200, description = "模型列表", body = HttpResult<ModelsResponse>),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn models_list_handler(State(state): State<AppState>) -> HttpResult<ModelsResponse> {
    // 使用配置中的支持模型列表
    let available_models = state.config.whisper.supported_models.clone();

    // 简化版本，假设默认模型已加载
    let loaded_models = vec![state.config.whisper.default_model.clone()];

    HttpResult::success(ModelsResponse {
        available_models,
        loaded_models,
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
        (status = 200, description = "转录成功", body = HttpResult<TranscriptionResponse>),
        (status = 400, description = "请求无效", body = String),
        (status = 413, description = "文件过大", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn transcribe_handler(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<HttpResult<TranscriptionResponse>, VoiceCliError> {
    // 使用临时目录进行流式处理
    let temp_dir = std::env::temp_dir();
    let task_id = generate_task_id();
    // 使用流式处理避免内存占用
    let (temp_file, _request) =
        extract_transcription_request_streaming(multipart, &task_id, &temp_dir).await?;

    // 使用转录引擎处理
    let transcription_engine = crate::services::TranscriptionEngine::new(state.model_service);

    let result = transcription_engine
        .transcribe_compatible_audio(
            transcription_engine.default_model(), // 使用配置中的默认模型
            &temp_file,
            transcription_engine.worker_timeout(), // 使用配置中的超时时间
        )
        .await?;

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
    let task_id = generate_task_id();
    info!("开始处理异步转录请求: {}", task_id);

    // 使用流式处理避免内存占用
    let (audio_file_path, request) = extract_transcription_request_streaming(
        multipart,
        &task_id,
        &state.audio_file_manager.storage_dir,
    )
    .await?;

    // 提交任务到队列 - 使用无锁管理器
    info!("开始提交任务到队列...");
    let mut storage = state.apalis_storage.clone();
    let manager = state.lock_free_apalis_manager.as_ref();

    // 如果请求中没有指定模型，使用配置中的默认模型
    let model = request
        .model
        .or_else(|| Some(state.config.whisper.default_model.clone()));

    info!("使用无锁 ApalisManager 提交任务...");
    let result = manager
        .submit_task(
            &mut storage,
            audio_file_path,
            request.filename,
            model,
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
    let task_id = generate_task_id();
    info!("开始处理URL异步转录请求: {} - URL: {}", task_id, request.url);

    // 从URL中提取文件名
    let filename = extract_filename_from_url(&request.url).unwrap_or_else(|| "audio_from_url".to_string());

    // 提交URL任务到队列 - 使用无锁管理器
    info!("开始提交URL任务到队列...");
    let mut storage = state.apalis_storage.clone();
    let manager = state.lock_free_apalis_manager.as_ref();

    // 如果请求中没有指定模型，使用配置中的默认模型
    let model = request
        .model
        .or_else(|| Some(state.config.whisper.default_model.clone()));

    info!("使用无锁 ApalisManager 提交URL任务...");
    let result = manager
        .submit_task_for_url(
            &mut storage,
            request.url,
            filename,
            model,
            request.response_format,
        )
        .await;
    info!("URL任务提交操作完成，结果: {:?}", result);
    let returned_task_id = result?;

    info!("URL异步转录任务提交成功: {}", returned_task_id);

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
        (status = 200, description = "状态获取成功", body = HttpResult<TaskStatusResponse>),
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
        (status = 200, description = "结果获取成功", body = HttpResult<TranscriptionResponse>),
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
/// POST /tasks/:task_id
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}",
    tag = "任务管理", 
    summary = "取消任务",
    description = "取消待处理或正在处理的转录任务",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "取消成功", body = HttpResult<CancelResponse>),  
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
        (status = 200, description = "重试成功", body = HttpResult<RetryResponse>),
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
    let mut storage = state.apalis_storage.clone();

    let retried = manager.retry_task(&mut storage, &task_id).await?;

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

/// 删除任务
/// DELETE /tasks/:task_id/delete
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}/delete",
    tag = "任务管理", 
    summary = "删除任务",
    description = "彻底删除任务数据，包括状态和结果",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "删除成功", body = HttpResult<DeleteResponse>),
        (status = 404, description = "任务不存在", body = String),
        (status = 500, description = "服务器错误", body = String)
    ),
)]
pub async fn delete_task_handler(
    State(state): State<AppState>,
    axum::extract::Path(task_id): axum::extract::Path<String>,
) -> Result<HttpResult<DeleteResponse>, VoiceCliError> {
    let manager = state.lock_free_apalis_manager.as_ref();

    let deleted = manager.delete_task(&task_id).await?;

    let response = DeleteResponse {
        task_id: task_id.clone(),
        deleted,
        message: if deleted {
            format!("任务 {} 已彻底删除", task_id)
        } else {
            format!("任务 {} 不存在", task_id)
        },
    };

    info!("任务删除操作: {} -> {}", task_id, response.message);
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
        (status = 200, description = "统计信息获取成功", body = HttpResult<TaskStatsResponse>),
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

/// URL转录请求数据
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UrlTranscriptionRequest {
    url: String,
    model: Option<String>,
    response_format: Option<String>,
}

/// 解析 multipart 请求，使用流式处理避免内存占用
async fn extract_transcription_request_streaming(
    mut multipart: Multipart,
    task_id: &str,
    temp_dir: &Path,
) -> Result<(PathBuf, TranscriptionRequest), VoiceCliError> {
    let mut filename: Option<String> = None;
    let mut model: Option<String> = None;
    let mut response_format: Option<String> = None;
    let mut audio_data_temp_file: Option<PathBuf> = None;

    // 收集所有字段信息
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| VoiceCliError::MultipartError(format!("解析 multipart 失败: {}", e)))?
    {
        let field_name = field.name().unwrap_or("unknown").to_string();

        match field_name.as_str() {
            "audio" => {
                // 立即处理音频字段，避免借用冲突
                filename = field.file_name().map(|s| s.to_string());

                // 创建临时文件
                let temp_filename = format!("task_{}.bin", task_id);
                let temp_file_path = temp_dir.join(&temp_filename);

                // 流式保存音频数据
                let file = tokio::fs::File::create(&temp_file_path)
                    .await
                    .map_err(|e| {
                        error!(
                            "[Task {}] 无法创建临时音频文件 '{}': {}",
                            task_id,
                            temp_file_path.display(),
                            e
                        );
                        VoiceCliError::Storage(format!(
                            "无法创建临时音频文件 '{}': {}",
                            temp_file_path.display(),
                            e
                        ))
                    })?;

                let mut writer = tokio::io::BufWriter::new(file);
                let mut reader = tokio_util::io::StreamReader::new(
                    field.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)),
                );

                let total_bytes = tokio::io::copy(&mut reader, &mut writer)
                    .await
                    .map_err(|e| {
                        error!("[Task {}] 流式复制音频文件数据失败: {}", task_id, e);
                        VoiceCliError::Storage(format!("流式复制音频文件数据失败: {}", e))
                    })?;

                writer.flush().await.map_err(|e| {
                    error!(
                        "[Task {}] 无法刷新数据到临时文件 '{}': {}",
                        task_id,
                        temp_file_path.display(),
                        e
                    );
                    VoiceCliError::Storage(format!(
                        "无法刷新数据到文件 '{}': {}",
                        temp_file_path.display(),
                        e
                    ))
                })?;

                info!(
                    "[Task {}] 成功接收音频文件: {} 字节 -> {}",
                    task_id,
                    total_bytes,
                    temp_file_path.display()
                );

                audio_data_temp_file = Some(temp_file_path);
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

    let temp_file_path =
        audio_data_temp_file.ok_or_else(|| VoiceCliError::MissingField("audio".to_string()))?;

    // 检查文件是否存在且有效
    let metadata = tokio::fs::metadata(&temp_file_path).await.map_err(|e| {
        error!(
            "[Task {}] 无法访问临时音频文件 '{}': {}",
            task_id,
            temp_file_path.display(),
            e
        );
        VoiceCliError::Storage(format!(
            "无法访问临时音频文件 '{}': {}",
            temp_file_path.display(),
            e
        ))
    })?;

    if metadata.len() == 0 {
        error!(
            "[Task {}] 接收到的音频文件为空: {}",
            task_id,
            temp_file_path.display()
        );
        return Err(VoiceCliError::Storage(format!(
            "音频文件为空: {}",
            temp_file_path.display()
        )));
    }

    // 探测文件真实格式
    let extension = match AudioFormatDetector::detect_format_from_path(&temp_file_path) {
        Ok(file_type) => file_type.extension().to_lowercase(),
        Err(_) => {
            warn!("[Task {}] 无法检测音频文件格式，使用默认扩展名", task_id);
            "bin".to_string()
        }
    };

    // 重命名为正确的扩展名
    let final_filename = format!("task_{}.{}", task_id, extension);
    let final_file_path = temp_dir.join(&final_filename);

    // 重命名文件
    tokio::fs::rename(&temp_file_path, &final_file_path)
        .await
        .map_err(|e| {
            error!(
                "[Task {}] 无法重命名临时文件 '{}' -> '{}': {}",
                task_id,
                temp_file_path.display(),
                final_file_path.display(),
                e
            );
            VoiceCliError::Storage(format!("重命名文件失败: {}", e))
        })?;

    info!(
        "[Task {}] 音频文件已重命名: {} -> {}",
        task_id,
        temp_file_path.display(),
        final_file_path.display()
    );

    // 使用原始文件名或生成的文件名
    let final_filename_str = filename.unwrap_or_else(|| final_filename.clone());

    let request = TranscriptionRequest {
        filename: final_filename_str,
        model,
        response_format,
    };

    Ok((final_file_path, request))
}

/// 从URL中提取文件名
fn extract_filename_from_url(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed_url| {
            parsed_url.path_segments()
                .and_then(|segments| segments.last())
                .map(|last_segment| last_segment.to_string())
        })
        .filter(|filename| !filename.is_empty())
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

/// TTS同步处理端点
/// POST /tts/sync
#[utoipa::path(
    post,
    path = "/tts/sync",
    tag = "TTS",
    summary = "同步文本转语音",
    description = "将文本转换为语音并直接返回音频文件",
    request_body = TtsSyncRequest,
    responses(
        (status = 200, description = "转换成功"),
        (status = 400, description = "请求参数错误"),
        (status = 500, description = "服务器内部错误")
    ),
)]
pub async fn tts_sync_handler(
    State(state): State<AppState>,
    Json(request): Json<TtsSyncRequest>,
) -> Result<axum::response::Response, HttpResult<String>> {
    let start_time = std::time::Instant::now();
    
    info!("收到TTS同步请求 - 文本长度: {}", request.text.len());

    // 验证文本长度
    if request.text.len() > state.config.tts.max_text_length {
        let error_msg = format!("文本长度超过限制 ({} > {})", 
            request.text.len(), state.config.tts.max_text_length);
        error!("{}", error_msg);
        return Ok(HttpResult::<String>::from(VoiceCliError::InvalidInput(error_msg)).into_response());
    }

    // 应用默认参数
    let mut processed_request = request.clone();
    processed_request.speed.get_or_insert(state.config.tts.default_speed);
    processed_request.pitch.get_or_insert(state.config.tts.default_pitch);
    processed_request.volume.get_or_insert(state.config.tts.default_volume);
    processed_request.format.get_or_insert("mp3".to_string());

    // 执行TTS合成
    match state.tts_service.synthesize_sync(processed_request).await {
        Ok(audio_file_path) => {
            let processing_time = start_time.elapsed();
            info!("TTS同步处理完成 - 耗时: {:?}", processing_time);

            // 读取音频文件并返回
            match tokio::fs::read(&audio_file_path).await {
                Ok(audio_data) => {
                    let content_type = match audio_file_path.extension()
                        .and_then(|ext| ext.to_str())
                        .unwrap_or("mp3") {
                        "wav" => "audio/wav",
                        "mp3" => "audio/mpeg",
                        _ => "audio/octet-stream",
                    };

                    let response = axum::response::Response::builder()
                        .status(200)
                        .header("Content-Type", content_type)
                        .header("Content-Length", audio_data.len())
                        .header("X-Processing-Time", format!("{:?}", processing_time))
                        .body(axum::body::Body::from(audio_data))
                        .unwrap();

                    Ok(response)
                }
                Err(e) => {
                    let error_msg = format!("读取音频文件失败: {}", e);
                    error!("{}", error_msg);
                    Ok(HttpResult::<String>::from(VoiceCliError::TtsError(error_msg)).into_response())
                }
            }
        }
        Err(e) => {
            let error_msg = format!("TTS合成失败: {}", e);
            error!("{}", error_msg);
            Ok(HttpResult::<String>::from(VoiceCliError::TtsError(error_msg)).into_response())
        }
    }
}

/// TTS异步处理端点
/// POST /api/v1/tasks/tts
#[utoipa::path(
    post,
    path = "/api/v1/tasks/tts",
    tag = "TTS",
    summary = "异步文本转语音",
    description = "提交TTS任务到队列，返回任务ID",
    request_body = TtsAsyncRequest,
    responses(
        (status = 202, description = "任务已接受", body = TtsTaskResponse),
        (status = 400, description = "请求参数错误", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
)]
pub async fn tts_async_handler(
    State(state): State<AppState>,
    Json(request): Json<TtsAsyncRequest>,
) -> HttpResult<TtsTaskResponse> {
    info!("收到TTS异步请求 - 文本长度: {}", request.text.len());

    // 验证文本长度
    if request.text.len() > state.config.tts.max_text_length {
        let error_msg = format!("文本长度超过限制 ({} > {})", 
            request.text.len(), state.config.tts.max_text_length);
        error!("{}", error_msg);
        return HttpResult::<String>::error("400".to_string(), error_msg);
    }

    // 应用默认参数
    let mut processed_request = request.clone();
    processed_request.speed.get_or_insert(state.config.tts.default_speed);
    processed_request.pitch.get_or_insert(state.config.tts.default_pitch);
    processed_request.volume.get_or_insert(state.config.tts.default_volume);
    processed_request.format.get_or_insert("mp3".to_string());

    // 创建异步任务
    match state.tts_service.create_async_task(processed_request).await {
        Ok(response) => {
            info!("TTS异步任务已创建 - ID: {}", response.task_id);
            HttpResult::success(response)
        }
        Err(e) => {
            let error_msg = format!("创建TTS异步任务失败: {}", e);
            error!("{}", error_msg);
            HttpResult::<TtsTaskResponse>::from(VoiceCliError::TtsError(error_msg))
        }
    }
}
