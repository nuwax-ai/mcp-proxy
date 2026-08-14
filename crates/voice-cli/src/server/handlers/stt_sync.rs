//! 同步转录端点（POST /transcribe）：multipart 流式接收 + 引擎分派同步推理。

use super::*;

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
    let task_id = crate::utils::generate_task_id();
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
