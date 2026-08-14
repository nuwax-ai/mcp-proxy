//! TTS 端点：同步合成、音色查询、异步任务提交/状态/音频下载。

use super::*;

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
