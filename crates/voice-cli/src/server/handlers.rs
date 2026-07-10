use crate::VoiceCliError;
use crate::models::config::TtsBackend;
use crate::models::{
    AsyncTaskResponse, CancelResponse, Config, DeleteResponse, HealthResponse, HttpResult,
    ModelsResponse, RetryResponse, SimpleTaskStatus, TaskStatsResponse, TaskStatus,
    TaskStatusResponse, TranscriptionResponse, TtsAsyncRequest, TtsSyncRequest, TtsTaskResponse,
    TtsTaskStatus,
};
use crate::services::{
    AudioFileManager, AudioFormatDetector, LockFreeApalisManager, MetadataExtractor, ModelService,
    TranscriptionTask, TtsApalisManager,
};
use crate::tts::{AudioFormat, TtsModelService, TtsOptions, resolve_reference};
use apalis_sql::sqlite::SqliteStorage;
use axum::extract::{Json, Multipart, Path as AxumPath, State};
use axum::response::IntoResponse;
use chrono::Utc;
use futures::TryStreamExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
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
    pub tts_model_service: Arc<TtsModelService>,
    pub tts_apalis_manager: Arc<TtsApalisManager>,
    pub tts_apalis_storage: SqliteStorage<crate::models::TtsTask>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // 初始化无锁 Apalis 管理器
        info!("Initializing the Lock-Free Apalis Task Manager");
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

        // 初始化 TTS 模型服务（v1 不自动下载；缺模型时请求阶段返回明确错误）
        let tts_model_service =
            Arc::new(TtsModelService::new(config.tts.engine.models_dir.clone()));

        // 初始化 TTS apalis 管理器（独立 DB，路径来自 config.tts.tasks_db_path）
        info!("Initializing TTS Apalis manager");
        let (tts_apalis_manager, tts_apalis_storage) = TtsApalisManager::new(
            config.task_management.clone(),
            config.tts.tasks_db_path.clone(),
        )
        .await?;
        let tts_apalis_manager = Arc::new(tts_apalis_manager);
        tts_apalis_manager
            .start_worker(
                tts_apalis_storage.clone(),
                config.tts.clone(),
                tts_model_service.clone(),
            )
            .await?;

        Ok(Self {
            config,
            model_service,
            lock_free_apalis_manager,
            apalis_storage,
            audio_file_manager,
            tts_model_service,
            tts_apalis_manager,
            tts_apalis_storage,
            start_time: SystemTime::now(),
        })
    }

    /// 优雅关闭
    pub async fn shutdown(&self) {
        info!("Close application state");

        // 优雅关闭 Apalis 管理器
        if let Err(e) = self.lock_free_apalis_manager.shutdown().await {
            warn!("Failed to close Apalis Manager: {}", e);
        }
        // 优雅关闭 TTS apalis 管理器
        if let Err(e) = self.tts_apalis_manager.shutdown().await {
            warn!("Failed to close TTS Apalis Manager: {}", e);
        }

        info!("Application status closed completed");
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

    // 真实已加载模型：STT + TTS 引擎池缓存（首次请求后才加载，故可能为空）
    let mut models_loaded = crate::stt::engine_pool::loaded_model_ids();
    models_loaded.extend(
        crate::tts::engine_pool::loaded_model_ids()
            .into_iter()
            .map(|m| format!("tts:{m}")),
    );

    HttpResult::success(HealthResponse {
        status: "healthy".to_string(),
        models_loaded,
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

    // 已加载 = STT 引擎池缓存实际存在的模型（首次请求后才加载，可能为空）
    let loaded_models = crate::stt::engine_pool::loaded_model_ids();

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
        description = "multipart/form-data：file(音频) + 可选 model/language/initial_prompt/engine。\
                       engine=whisper|sensevoice|fireredasr2|funasrnano|qwen3asr 覆盖后端（A/B 对比）；\
                       sensevoice/fireredasr2/funasrnano/qwen3asr 仅批量、原生简体中文",
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
    let (temp_file, request) =
        extract_transcription_request_streaming(multipart, &task_id, &temp_dir).await?;

    // 提取音视频元数据
    let metadata = match MetadataExtractor::extract_metadata(&temp_file).await {
        Ok(meta) => {
            info!(
                "Audio and video metadata successfully extracted: {}",
                crate::services::MetadataExtractor::get_format_description(&meta)
            );
            // 转换为models::request::AudioVideoMetadata
            Some(crate::models::request::AudioVideoMetadata {
                format: meta.format,
                container_format: meta.container_format,
                duration_seconds: meta.duration_seconds,
                file_size_bytes: meta.file_size_bytes,
                audio_codec: meta.audio_codec,
                sample_rate: meta.sample_rate,
                channels: meta.channels,
                audio_bitrate: meta.audio_bitrate,
                has_video: meta.has_video,
                video_codec: meta.video_codec,
                width: meta.width,
                height: meta.height,
                video_bitrate: meta.video_bitrate,
                frame_rate: meta.frame_rate,
                bitrate: meta.bitrate,
                creation_time: meta.creation_time,
            })
        }
        Err(e) => {
            warn!("Failed to extract metadata, using default value: {}", e);
            None
        }
    };

    // 后端引擎：请求字段优先（A/B 对比用），回退 config.whisper.engine.backend
    let engine_cfg = &state.config.whisper.engine;
    let backend = request.engine.unwrap_or(engine_cfg.backend);
    let tr_pool_size = engine_cfg.pool_size; // transcribe-rs dyn 池大小（sherpa 走 load_params.pool_size）
    let output_script = engine_cfg.output_script;

    // 模型解析 + ensure + 构造分派产物（按 backend 分派）。两库引擎统一为 SttInvocation：
    // transcribe-rs（whisper/sensevoice）走 dyn 池；sherpa-onnx（fireredasr2/funasrnano/qwen3asr）走独立池。
    let invocation: crate::stt::SttInvocation = match backend {
        crate::models::config::SttBackend::Whisper => {
            // 模型 id：请求字段优先，回退到 config.whisper.default_model（与异步 handler 一致）
            let model_id = request
                .model
                .clone()
                .or_else(|| Some(state.config.whisper.default_model.clone()))
                .unwrap_or_default();
            // ensure_model：模型缺失时自动下载（接入 HTTP，修复旧版 auto_download 形同虚设）
            state.model_service.ensure_model(&model_id).await?;
            let model_path = state.model_service.get_model_path(&model_id)?;
            crate::stt::SttInvocation::TranscribeRs {
                model_id,
                spec: crate::stt::SttEngineSpec::Whisper { model_path },
            }
        }
        crate::models::config::SttBackend::SenseVoice => {
            // SenseVoice：从 config.whisper.engine.sensevoice 解析目录 + 量化（host 预置，无自动下载）
            #[cfg(feature = "sensevoice")]
            {
                let models_dir = state.config.whisper.models_dir.clone();
                let sv_cfg = &engine_cfg.sensevoice;
                let model_dir = crate::stt::sensevoice::resolve_model_dir(&models_dir, sv_cfg);
                let quantization =
                    crate::stt::sensevoice::parse_quantization(&sv_cfg.quantization)?;
                let model_id = model_dir.display().to_string();
                crate::stt::SttInvocation::TranscribeRs {
                    model_id,
                    spec: crate::stt::SttEngineSpec::SenseVoice {
                        model_dir,
                        quantization,
                    },
                }
            }
            #[cfg(not(feature = "sensevoice"))]
            {
                let _ = request.model;
                return Err(VoiceCliError::TranscriptionFailed(
                    "backend=sensevoice 但未启用 sensevoice feature（请用 --features sensevoice 编译）".into(),
                ));
            }
        }
        // sherpa-onnx 三引擎（FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR）：仅批量，走独立 sherpa 池
        crate::models::config::SttBackend::FireRedAsr2
        | crate::models::config::SttBackend::FunAsrNano
        | crate::models::config::SttBackend::Qwen3Asr => {
            use crate::stt::sherpa_model_paths::{
                DEFAULT_FIREREDASR2_DIR, DEFAULT_FUNASRNANO_DIR, DEFAULT_QWEN3ASR_DIR,
                resolve_model_dir,
            };
            let sh = &engine_cfg.sherpa;
            let models_dir = std::path::Path::new(&state.config.whisper.models_dir);
            // 按 backend 解析模型目录 + 组 kind（hotwords / max_new_tokens 透传）
            let (model_dir, kind) = match backend {
                crate::models::config::SttBackend::FireRedAsr2 => (
                    resolve_model_dir(
                        models_dir,
                        "fireredasr2",
                        DEFAULT_FIREREDASR2_DIR,
                        sh.fireredasr2.model_dir.as_deref(),
                    ),
                    crate::stt::SherpaAsrKind::FireRedAsr2,
                ),
                crate::models::config::SttBackend::FunAsrNano => (
                    resolve_model_dir(
                        models_dir,
                        "funasrnano",
                        DEFAULT_FUNASRNANO_DIR,
                        sh.funasrnano.model_dir.as_deref(),
                    ),
                    crate::stt::SherpaAsrKind::FunAsrNano {
                        hotwords: sh.funasrnano.hotwords.clone(),
                    },
                ),
                crate::models::config::SttBackend::Qwen3Asr => (
                    resolve_model_dir(
                        models_dir,
                        "qwen3asr",
                        DEFAULT_QWEN3ASR_DIR,
                        sh.qwen3asr.model_dir.as_deref(),
                    ),
                    crate::stt::SherpaAsrKind::Qwen3Asr {
                        hotwords: sh.qwen3asr.hotwords.clone(),
                        max_new_tokens: sh.qwen3asr.max_new_tokens,
                    },
                ),
                // Whisper / SenseVoice 由前置独立臂处理，逻辑不可达
                _ => unreachable!("sherpa 分派臂已覆盖全部 sherpa backend"),
            };
            let model_id = model_dir.display().to_string();
            let load_params = crate::stt::SherpaAsrLoadParams {
                model_dir,
                kind,
                provider: sh.provider.clone(),
                num_threads: sh.num_threads,
                pool_size: sh.pool_size,
                debug: sh.debug,
            };
            crate::stt::SttInvocation::Sherpa {
                model_id,
                load_params,
            }
        }
    };

    // STT 参数：请求字段优先，回退到 config.whisper.engine 默认（P1 透传）
    let opt_language = request
        .language
        .or_else(|| engine_cfg.default_language.clone());
    let opt_initial_prompt = request
        .initial_prompt
        .or_else(|| engine_cfg.default_initial_prompt.clone());

    // 同步推理走 spawn_blocking（两库引擎都是同步阻塞 C 调用，不能阻塞 tokio reactor）。
    // 闭包按 SttInvocation 分两条路径，共享 to_whisper_samples，统一返回已 map 的 TranscriptionResponse。
    let temp_file_for_blocking = temp_file.clone();
    let mut response = tokio::task::spawn_blocking(
        move || -> std::result::Result<crate::models::request::TranscriptionResponse, crate::stt::SttError>
    {
            // ffmpeg-sidecar 转 16k/mono/s16le → f32 samples（引擎无关：whisper/sensevoice/sherpa 都要 16k mono）
            let samples = crate::stt::audio::to_whisper_samples(&temp_file_for_blocking)?;
            match invocation {
                crate::stt::SttInvocation::TranscribeRs { model_id, spec } => {
                    // 进程级 dyn 池：whisper/sensevoice 统一 Box<dyn SpeechModel>
                    let key = crate::stt::EngineKey::new(&model_id);
                    let pool = crate::stt::get_or_init_engine(key, spec, tr_pool_size)?;
                    let inst = pool.pick();
                    let mut guard = inst.lock().unwrap_or_else(|p| p.into_inner());
                    let opts = crate::stt::SttTranscribeOptions {
                        language: opt_language,
                        initial_prompt: opt_initial_prompt,
                        ..Default::default()
                    };
                    // 共享 trait 方法：whisper + sensevoice 都实现 SpeechModel::transcribe
                    let result = guard.transcribe(&samples, &opts.to_transcribe_options())?;
                    Ok(crate::stt::map_transcription_result(result, output_script))
                }
                crate::stt::SttInvocation::Sherpa {
                    model_id,
                    load_params,
                } => {
                    // 独立 sherpa-onnx 池：Pool<OfflineRecognizer>（FireRedASR2/Fun-ASR-Nano/Qwen3-ASR）
                    // FireRedASR2-AED 无标点 → 加 CT-Transformer 标点（已 init 才生效，否则透传）；
                    // Fun/Qwen 自带 LLM 标点，跳过（is_fireredasr2=false）
                    let is_fireredasr2 =
                        matches!(load_params.kind, crate::stt::SherpaAsrKind::FireRedAsr2);
                    let key = crate::stt::EngineKey::new(&model_id);
                    let pool = crate::stt::sherpa_get_or_init_engine(key, load_params)?;
                    let inst = pool.pick();
                    let guard = inst.lock().unwrap_or_else(|p| p.into_inner());
                    let mut result = crate::stt::sherpa_recognize(&guard, &samples)?;
                    if is_fireredasr2 {
                        result.text = crate::stt::sherpa_punc::add_punct(&result.text);
                    }
                    Ok(crate::stt::map_sherpa_recognition_result(result, output_script))
                }
            }
        },
    )
    .await
    .map_err(|e| VoiceCliError::TranscriptionFailed(format!("转录任务 join 失败: {e}")))??;

    // 设置元数据和时长
    if let Some(meta) = &metadata {
        response.duration = Some(meta.duration_seconds as f32);
        response.metadata = Some(meta.clone());
    }

    info!(
        "Synchronous transcription completed: {} characters",
        response.text.len()
    );

    // 清理临时文件 - 使用异步任务确保即使出错也不影响响应
    let cleanup_file = temp_file.clone();
    info!("Temporary file: {}", temp_file.display());
    tokio::spawn(async move {
        match tokio::fs::remove_file(&cleanup_file).await {
            Ok(_) => info!("Cleaned temporary files: {}", cleanup_file.display()),
            Err(e) => warn!(
                "Failed to clean up temporary files {}: {}",
                cleanup_file.display(),
                e
            ),
        }
    });

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
    let task_id = generate_task_id();
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
            info!(
                "Obtaining task status successfully: {} -> {:?}",
                task_id, status
            );
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
            warn!("Task does not exist: {}", task_id);
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

    match manager
        .get_task_result(&task_id, state.config.whisper.engine.output_script)
        .await?
    {
        Some(result) => {
            info!(
                "Successful acquisition of task results: {} -> {} characters",
                task_id,
                result.text.len()
            );
            Ok(HttpResult::success(result))
        }
        None => {
            warn!("Task result not available: {}", task_id);
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

    info!(
        "Task cancellation operation: {} -> {}",
        task_id, response.message
    );
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

    info!("Task retry operation: {} -> {}", task_id, response.message);
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

    info!(
        "Task deletion operation: {} -> {}",
        task_id, response.message
    );
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

    info!("Get task statistics: Total {} tasks", stats.total_tasks);
    Ok(HttpResult::success(stats))
}

/// GET /api/v1/tasks/tts/stats — TTS 任务统计（对称 STT /api/v1/tasks/stats）
#[utoipa::path(
    get,
    path = "/api/v1/tasks/tts/stats",
    tag = "任务管理",
    summary = "TTS 任务统计",
    description = "返回 TTS 异步任务的总数/各状态计数/失败 task_id/平均处理时间",
    responses(
        (status = 200, description = "TTS 任务统计", body = HttpResult<TaskStatsResponse>),
        (status = 500, description = "服务器错误", body = String)
    )
)]
pub async fn tts_tasks_stats_handler(
    State(state): State<AppState>,
) -> Result<HttpResult<TaskStatsResponse>, VoiceCliError> {
    let stats = state.tts_apalis_manager.get_tasks_stats().await?;
    Ok(HttpResult::success(stats))
}

// ===== 辅助函数 =====

/// 转录请求数据
#[derive(Debug)]
struct TranscriptionRequest {
    filename: String,
    model: Option<String>,
    response_format: Option<String>,
    /// 目标语种（BCP-47，如 `"en"`/`"zh"`；`None` = 自动检测）
    language: Option<String>,
    /// 初始提示，给模型领域上下文（提升专有词 / 风格准确率）
    initial_prompt: Option<String>,
    /// STT 后端覆盖：`whisper` / `sensevoice`（仅同步 /transcribe 生效，用于 A/B 对比；
    /// `None` = 用 config.whisper.engine.backend）。异步任务忽略此字段（config-only）。
    engine: Option<crate::models::config::SttBackend>,
}

/// URL转录请求数据
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UrlTranscriptionRequest {
    url: String,
    model: Option<String>,
    response_format: Option<String>,
    /// 目标语种（BCP-47，如 `"en"`/`"zh"`；`None` = 自动检测）
    language: Option<String>,
    /// 初始提示，给模型领域上下文（提升专有词 / 风格准确率）
    initial_prompt: Option<String>,
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
    let mut language: Option<String> = None;
    let mut initial_prompt: Option<String> = None;
    let mut engine: Option<crate::models::config::SttBackend> = None;
    let mut audio_data_temp_file: Option<PathBuf> = None;

    // 收集所有字段信息
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| VoiceCliError::MultipartError(format!("解析 multipart 失败: {}", e)))?
    {
        let field_name = field.name().unwrap_or("unknown").to_string();

        match field_name.as_str() {
            "file" | "audio" => {
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
                            "[Task {}] Unable to create temporary audio file '{}': {}",
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
                let mut reader =
                    tokio_util::io::StreamReader::new(field.map_err(std::io::Error::other));

                let total_bytes = tokio::io::copy(&mut reader, &mut writer)
                    .await
                    .map_err(|e| {
                        error!("[Task {}] Failed to stream audio file data: {}", task_id, e);
                        VoiceCliError::Storage(format!("流式复制音频文件数据失败: {}", e))
                    })?;

                writer.flush().await.map_err(|e| {
                    error!(
                        "[Task {}] Unable to refresh data to temporary file '{}': {}",
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
                    "[Task {}] Successfully received audio file: {} bytes -> {}",
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
            "language" => {
                language = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("解析 language 参数失败: {}", e))
                })?);
            }
            "initial_prompt" => {
                initial_prompt = Some(field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("解析 initial_prompt 参数失败: {}", e))
                })?);
            }
            "engine" => {
                let v = field.text().await.map_err(|e| {
                    VoiceCliError::MultipartError(format!("解析 engine 参数失败: {}", e))
                })?;
                engine = Some(match v.trim().to_ascii_lowercase().as_str() {
                    "whisper" => crate::models::config::SttBackend::Whisper,
                    "sensevoice" => crate::models::config::SttBackend::SenseVoice,
                    "fireredasr2" => crate::models::config::SttBackend::FireRedAsr2,
                    "funasrnano" => crate::models::config::SttBackend::FunAsrNano,
                    "qwen3asr" => crate::models::config::SttBackend::Qwen3Asr,
                    other => {
                        return Err(VoiceCliError::MultipartError(format!(
                            "未知 engine 参数: {other}（支持 whisper / sensevoice / fireredasr2 / funasrnano / qwen3asr）"
                        )));
                    }
                });
            }
            _ => {
                warn!("Ignore unknown fields: {}", field_name);
            }
        }
    }

    let temp_file_path =
        audio_data_temp_file.ok_or_else(|| VoiceCliError::MissingField("audio".to_string()))?;

    // 检查文件是否存在且有效
    let metadata = tokio::fs::metadata(&temp_file_path).await.map_err(|e| {
        error!(
            "[Task {}] Unable to access temporary audio file '{}': {}",
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
            "[Task {}] The received audio file is empty: {}",
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
        Ok(Some(file_type)) => file_type.extension().to_lowercase(),
        Ok(None) => {
            warn!(
                "[Task {}] Unable to detect audio file format, try using file extension",
                task_id
            );
            // 尝试使用文件扩展名作为后备
            if let Some(ext) = temp_file_path.extension().and_then(|e| e.to_str()) {
                ext.to_lowercase()
            } else {
                "bin".to_string()
            }
        }
        Err(_) => {
            warn!(
                "[Task {}] Error detecting file format, using default extension",
                task_id
            );
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
                "[Task {}] Unable to rename temporary file '{}' -> '{}': {}",
                task_id,
                temp_file_path.display(),
                final_file_path.display(),
                e
            );
            VoiceCliError::Storage(format!("重命名文件失败: {}", e))
        })?;

    info!(
        "[Task {}] The audio file has been renamed: {} -> {}",
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
        language,
        initial_prompt,
        engine,
    };

    Ok((final_file_path, request))
}

/// 从URL中提取文件名
fn extract_filename_from_url(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|parsed_url| {
            parsed_url
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(|last_segment| last_segment.to_string())
        })
        .filter(|filename| !filename.is_empty())
}

/// 生成任务 ID - 使用统一的工具函数
fn generate_task_id() -> String {
    crate::utils::generate_task_id()
}

/// TTS 同步合成端点（sherpa-onnx Kokoro）。
/// POST /api/v1/tts
///
/// 返回二进制音频（wav / pcm_s16le）。失败时返回 HttpResult JSON 错误。
#[utoipa::path(
    post,
    path = "/api/v1/tts",
    tag = "TTS",
    summary = "同步文本转语音（sherpa-onnx Kokoro）",
    description = "将文本合成为语音，直接返回二进制音频（wav / pcm_s16le）",
    request_body = TtsSyncRequest,
    responses(
        (status = 200, description = "合成成功，返回二进制音频", content_type = "audio/wav"),
        (status = 400, description = "请求参数错误 / 模型未找到", body = HttpResult<String>),
        (status = 500, description = "合成 / 编码失败", body = HttpResult<String>),
        (status = 503, description = "TTS 未启用", body = HttpResult<String>)
    ),
)]
pub async fn tts_sync_handler(
    State(state): State<AppState>,
    Json(request): Json<TtsSyncRequest>,
) -> Result<axum::response::Response, HttpResult<String>> {
    let start_time = std::time::Instant::now();

    // Fail Fast：TTS 未启用直接 503
    if !state.config.tts.enabled {
        let msg = "TTS service is disabled (config.tts.enabled=false)".to_string();
        return Ok(HttpResult::<String>::from(VoiceCliError::InvalidInput(msg)).into_response());
    }

    // 验证文本非空 + 长度
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return Ok(HttpResult::<String>::from(VoiceCliError::InvalidInput(
            "text 不能为空".to_string(),
        ))
        .into_response());
    }
    if text.len() > state.config.tts.max_text_length {
        let error_msg = format!(
            "文本长度超过限制 ({} > {})",
            text.len(),
            state.config.tts.max_text_length
        );
        return Ok(
            HttpResult::<String>::from(VoiceCliError::InvalidInput(error_msg)).into_response(),
        );
    }

    info!(text_len = text.len(), "TTS sync request received");

    let engine = &state.config.tts.engine;
    let model_id = request
        .model
        .clone()
        .unwrap_or_else(|| engine.default_model.clone());

    // ensure_model：缺失即 400，附手动放置指引（Fail Fast，早于 spawn_blocking）
    if let Err(e) = state.tts_model_service.ensure_model(&model_id, engine) {
        return Ok(HttpResult::<String>::from(VoiceCliError::from(e)).into_response());
    }

    // 解析输出格式：请求优先，回退到 tts.streaming.default_format（未知默认 wav）
    let format = match request.format.as_deref() {
        Some(f) => AudioFormat::parse(f),
        None => AudioFormat::parse(&state.config.tts.streaming.default_format),
    };
    // 池化参数（model-level；length_scale 仅 Kokoro 首次加载生效）
    let length_scale = request.length_scale.unwrap_or(engine.default_length_scale);
    // per-request 参数（reference 解码 CPU 密集，随 synth 一起放 spawn_blocking）
    let sid = request.sid.unwrap_or(engine.default_sid);
    let speed = request.speed.unwrap_or(engine.default_speed);
    let backend = engine.backend;
    let num_steps = engine.zipvoice.num_steps;
    let voice = request.voice.clone();
    let reference_audio = request.reference_audio.clone();
    let reference_text = request.reference_text.clone();

    // 同步合成走 spawn_blocking（sherpa-onnx 是同步阻塞 C 调用）
    let model_svc = state.tts_model_service.clone();
    // clone Arc<Config>（廉价）而非 TtsEngineConfig（含 voices Vec），避免每请求复制 voices
    let config = state.config.clone();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<_, VoiceCliError> {
        let opts = TtsOptions {
            sid,
            speed,
            num_steps,
            reference: match backend {
                TtsBackend::Kokoro => None,
                // resolve_reference：reference_audio 优先 → voice 预置 → 回退首个预置 → Err
                TtsBackend::ZipVoice => resolve_reference(
                    voice.as_deref(),
                    reference_audio.as_deref(),
                    reference_text.as_deref(),
                )?,
            },
            ..Default::default()
        };
        let (bytes, sample_rate, _n) = crate::tts::synth_to_bytes(
            &model_svc,
            &model_id,
            &config.tts.engine,
            &text,
            &opts,
            length_scale,
            format,
        )?;
        Ok((bytes, format, sample_rate))
    })
    .await
    .map_err(|e| VoiceCliError::TtsError(format!("TTS 任务 join 失败: {e}")))??;

    let (bytes, format, _sample_rate) = result;
    let processing_time = start_time.elapsed();
    info!(
        duration_ms = processing_time.as_millis() as u64,
        bytes = bytes.len(),
        "TTS sync completed"
    );

    let response = axum::response::Response::builder()
        .status(200)
        .header("Content-Type", format.content_type())
        .header("Content-Length", bytes.len())
        .header("X-Processing-Time", format!("{:?}", processing_time))
        .body(axum::body::Body::from(bytes))
        .map_err(|e| VoiceCliError::TtsError(format!("构建响应失败: {e}")))?;
    Ok(response)
}

/// TTS 音色列表端点。
/// GET /api/v1/tts/voices
///
/// 返回当前模型的音色数（`num_speakers()`）。具体音色名映射 v1 暂不维护（用 sid 索引）。
#[utoipa::path(
    get,
    path = "/api/v1/tts/voices",
    tag = "TTS",
    summary = "查询 TTS 可用音色数",
    responses(
        (status = 200, description = "返回音色数（JSON: {model, num_speakers}）", content_type = "application/json"),
        (status = 503, description = "TTS 未启用 / 模型未就绪", body = HttpResult<String>)
    ),
)]
pub async fn tts_voices_handler(
    State(state): State<AppState>,
) -> Result<axum::response::Response, HttpResult<String>> {
    if !state.config.tts.enabled {
        let msg = "TTS service is disabled".to_string();
        return Ok(HttpResult::<String>::from(VoiceCliError::InvalidInput(msg)).into_response());
    }
    // clone Arc<Config>（廉价）而非 TtsEngineConfig（含 voices Vec），避免每请求复制 voices
    let config = state.config.clone();
    let model_id = config.tts.engine.default_model.clone();
    if let Err(e) = state
        .tts_model_service
        .ensure_model(&model_id, &config.tts.engine)
    {
        return Ok(HttpResult::<String>::from(VoiceCliError::from(e)).into_response());
    }

    // Kokoro：num_speakers（加载引擎取）；ZipVoice：预置音色名（克隆引擎无内置 speaker）
    let model_svc = state.tts_model_service.clone();
    let backend = config.tts.engine.backend;
    let body =
        tokio::task::spawn_blocking(move || -> std::result::Result<serde_json::Value, VoiceCliError> {
            Ok(match backend {
                TtsBackend::Kokoro => {
                    let inst = crate::tts::acquire_instance(
                        &model_svc,
                        &model_id,
                        &config.tts.engine,
                        config.tts.engine.default_length_scale,
                    )?;
                    let guard = inst.lock().unwrap_or_else(|p| p.into_inner());
                    serde_json::json!({ "model": &model_id, "backend": "kokoro", "num_speakers": guard.num_speakers() })
                }
                TtsBackend::ZipVoice => {
                    let profiles = crate::tts::reference_profile_names();
                    serde_json::json!({
                        "model": &model_id, "backend": "zipvoice", "num_speakers": profiles.len(),
                        "profiles": profiles,
                        "hint": "克隆引擎：请求传 voice:<预置名> 或 reference_audio(base64 WAV) 动态克隆"
                    })
                }
            })
        })
        .await
        .map_err(|e| VoiceCliError::TtsError(format!("TTS voices join 失败: {e}")))??;

    let response = axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .map_err(|e| VoiceCliError::TtsError(format!("构建响应失败: {e}")))?;
    Ok(response)
}

/// TTS 异步任务提交端点。
/// POST /api/v1/tasks/tts
#[utoipa::path(
    post,
    path = "/api/v1/tasks/tts",
    tag = "TTS",
    summary = "异步文本转语音（提交任务）",
    request_body = TtsAsyncRequest,
    responses(
        (status = 202, description = "任务已接受", body = TtsTaskResponse),
        (status = 400, description = "请求参数错误 / 模型未找到", body = HttpResult<String>),
        (status = 503, description = "TTS 未启用", body = HttpResult<String>)
    ),
)]
pub async fn tts_async_handler(
    State(state): State<AppState>,
    Json(request): Json<TtsAsyncRequest>,
) -> HttpResult<TtsTaskResponse> {
    if !state.config.tts.enabled {
        return HttpResult::<TtsTaskResponse>::from(VoiceCliError::InvalidInput(
            "TTS service is disabled".to_string(),
        ));
    }
    let text = request.text.trim().to_string();
    if text.is_empty() {
        return HttpResult::<TtsTaskResponse>::from(VoiceCliError::InvalidInput(
            "text 不能为空".to_string(),
        ));
    }
    if text.len() > state.config.tts.max_text_length {
        return HttpResult::<TtsTaskResponse>::from(VoiceCliError::InvalidInput(format!(
            "文本长度超限 ({} > {})",
            text.len(),
            state.config.tts.max_text_length
        )));
    }

    let engine = &state.config.tts.engine;
    let model_id = request
        .model
        .clone()
        .unwrap_or_else(|| engine.default_model.clone());
    // ensure_model：缺失即拒（Fail Fast，不进队列空跑）
    if let Err(e) = state.tts_model_service.ensure_model(&model_id, engine) {
        return HttpResult::<TtsTaskResponse>::from(VoiceCliError::from(e));
    }

    let format = request
        .format
        .as_deref()
        .unwrap_or(&state.config.tts.streaming.default_format)
        .to_string();
    let task = crate::models::TtsTask {
        task_id: crate::utils::generate_task_id(),
        text,
        sid: request.sid.unwrap_or(engine.default_sid),
        speed: request.speed.unwrap_or(engine.default_speed),
        length_scale: request.length_scale.unwrap_or(engine.default_length_scale),
        language: request.language,
        format: format.clone(),
        model: model_id,
        voice: request.voice,
        reference_audio: request.reference_audio,
        reference_text: request.reference_text,
        created_at: Utc::now(),
    };
    let estimated = task.estimate_duration_secs();
    let task_id = task.task_id.clone();

    let mut storage = state.tts_apalis_storage.clone();
    if let Err(e) = state
        .tts_apalis_manager
        .submit_task(&mut storage, task)
        .await
    {
        return HttpResult::<TtsTaskResponse>::from(e);
    }
    info!(%task_id, "TTS async task submitted");
    HttpResult::success(crate::models::TtsTaskResponse {
        task_id,
        message: "TTS 任务已提交".to_string(),
        estimated_duration: Some(estimated),
    })
}

/// TTS 异步任务状态查询。
/// GET /api/v1/tasks/tts/{task_id}
#[utoipa::path(
    get,
    path = "/api/v1/tasks/tts/{task_id}",
    tag = "TTS",
    summary = "查询 TTS 任务状态",
    params(("task_id" = String, Path, description = "任务 id")),
    responses(
        (status = 200, description = "任务状态", body = TtsTaskStatus),
        (status = 404, description = "任务不存在", body = HttpResult<String>)
    ),
)]
pub async fn tts_task_status_handler(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
) -> HttpResult<crate::models::TtsTaskStatus> {
    match state.tts_apalis_manager.get_task_status(&task_id).await {
        Ok(Some(s)) => HttpResult::success(s),
        Ok(None) => HttpResult::<crate::models::TtsTaskStatus>::from(VoiceCliError::ModelNotFound(
            format!("TTS 任务不存在: {task_id}"),
        )),
        Err(e) => HttpResult::<crate::models::TtsTaskStatus>::from(e),
    }
}

/// TTS 异步任务音频下载。
/// GET /api/v1/tasks/tts/{task_id}/audio
#[utoipa::path(
    get,
    path = "/api/v1/tasks/tts/{task_id}/audio",
    tag = "TTS",
    summary = "下载 TTS 任务音频（Completed 状态可下载）",
    params(("task_id" = String, Path, description = "任务 id")),
    responses(
        (status = 200, description = "音频二进制", content_type = "audio/wav"),
        (status = 404, description = "任务不存在 / 音频未就绪", body = HttpResult<String>)
    ),
)]
pub async fn tts_task_audio_handler(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
) -> Result<axum::response::Response, HttpResult<String>> {
    let status = match state.tts_apalis_manager.get_task_status(&task_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Ok(
                HttpResult::<String>::from(VoiceCliError::ModelNotFound(format!(
                    "TTS 任务不存在: {task_id}"
                )))
                .into_response(),
            );
        }
        Err(e) => return Ok(HttpResult::<String>::from(e).into_response()),
    };
    let path = match status {
        crate::models::TtsTaskStatus::Completed {
            audio_file_path, ..
        } => audio_file_path,
        other => {
            let stage = match &other {
                crate::models::TtsTaskStatus::Pending { .. } => "Pending",
                crate::models::TtsTaskStatus::Processing { .. } => "Processing",
                crate::models::TtsTaskStatus::Failed { .. } => "Failed",
                crate::models::TtsTaskStatus::Cancelled { .. } => "Cancelled",
                crate::models::TtsTaskStatus::Completed { .. } => "Completed",
            };
            return Ok(
                HttpResult::<String>::from(VoiceCliError::InvalidInput(format!(
                    "TTS 任务尚未完成（当前状态: {stage}）"
                )))
                .into_response(),
            );
        }
    };
    let path = PathBuf::from(path);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let content_type = match path.extension().and_then(|e| e.to_str()).unwrap_or("wav") {
                "pcm" => "audio/pcm",
                _ => "audio/wav",
            };
            let resp = axum::response::Response::builder()
                .status(200)
                .header("Content-Type", content_type)
                .header("Content-Length", bytes.len())
                .body(axum::body::Body::from(bytes))
                .map_err(|e| VoiceCliError::TtsError(format!("构建音频响应失败: {e}")))?;
            Ok(resp)
        }
        Err(e) => Ok(HttpResult::<String>::from(VoiceCliError::TtsError(format!(
            "读取音频文件失败: {e}"
        )))
        .into_response()),
    }
}
