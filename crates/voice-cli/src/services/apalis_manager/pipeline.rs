//! 转录流水线：worker 编排 + 三步骤（音频预处理 / STT 转录 / 结果格式化落库）。

use super::*;

/// 步骤 1: 音频预处理（包含URL下载）
async fn audio_preprocessing_step(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<AudioProcessedTask, Error> {
    info!("Step 1 - Audio Preprocessing: {}", task.task_id);

    // 更新状态为处理中
    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Processing {
            stage: crate::models::ProcessingStage::AudioFormatDetection,
            started_at: Utc::now(),
            progress_details: None,
        },
    )
    .await?;

    let audio_file_path = if task.task_type == TaskType::UrlDownload {
        // URL下载任务：下载音频文件
        info!(
            "Download URL audio file: {} - URL: {:?}",
            task.task_id, task.url
        );

        if let Some(url) = task.url {
            let downloaded_path =
                download_audio_from_url(&url, &task.task_id, &ctx.audio_file_manager.storage_dir)
                    .await
                    .map_err(|e| {
                        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                            format!("下载URL音频文件失败: {}", e),
                        ))))
                    })?;

            // 检测文件真实格式并重命名
            let final_audio_path = detect_and_rename_audio_file(&downloaded_path, &task.task_id)
                .await
                .map_err(|e| {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                        format!("检测音频文件格式失败: {}", e),
                    ))))
                })?;

            // 更新数据库中的文件路径
            update_task_file_path_in_db(&task.task_id, &final_audio_path, &ctx)
                .await
                .map_err(|e| {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                        format!("更新数据库文件路径失败: {}", e),
                    ))))
                })?;

            final_audio_path
        } else {
            return Err(Error::Abort(std::sync::Arc::new(Box::new(
                std::io::Error::other(format!("URL任务缺少URL地址: {}", task.task_id)),
            ))));
        }
    } else {
        // 文件上传任务：直接使用现有文件路径
        info!(
            "Process file upload task: {} - File: {:?}",
            task.task_id, task.audio_file_path
        );

        // 读取并验证音频文件
        let _audio_data = tokio::fs::read(&task.audio_file_path).await.map_err(|e| {
            Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                format!("读取音频文件失败: {}", e),
            ))))
        })?;

        // 确保数据库中的文件路径是正确的（文件上传任务）
        update_task_file_path_in_db(&task.task_id, &task.audio_file_path, &ctx)
            .await
            .map_err(|e| {
                Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                    format!("更新数据库文件路径失败: {}", e),
                ))))
            })?;

        task.audio_file_path
    };

    // 音频预处理完成，进入下一步
    let processed_task = AudioProcessedTask {
        task_id: task.task_id.clone(),
        processed_audio_path: audio_file_path,
        original_filename: task.original_filename,
        model: task.model,
        response_format: task.response_format,
        language: task.language,
        initial_prompt: task.initial_prompt,
        created_at: task.created_at,
    };

    info!("Audio preprocessing completed: {}", task.task_id);
    Ok(processed_task)
}

/// 步骤 2: Whisper 转录
async fn transcription_step(
    task: AudioProcessedTask,
    ctx: Data<StepContext>,
) -> Result<TranscriptionCompletedTask, Error> {
    info!("Step 2 - Whisper Transcription: {}", task.task_id);

    // 更新状态为转录中
    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Processing {
            stage: crate::models::ProcessingStage::WhisperTranscription,
            started_at: Utc::now(),
            progress_details: None,
        },
    )
    .await?;

    // 提取音视频元数据
    let metadata = match MetadataExtractor::extract_metadata(&task.processed_audio_path).await {
        Ok(meta) => {
            info!(
                "[Task {}] Successfully extracted audio and video metadata: {}",
                task.task_id,
                MetadataExtractor::get_format_description(&meta)
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
            warn!("[Task {}] Failed to extract metadata: {}", task.task_id, e);
            None
        }
    };

    // 后端引擎（config-only：异步任务不做 per-task override，避免任务 schema 复杂化）
    let cfg = ctx.model_service.config();
    let backend = cfg.whisper.engine.backend;
    let tr_pool_size = cfg.whisper.engine.pool_size; // transcribe-rs dyn 池（sherpa 走 load_params.pool_size）
    // worker 存繁体原样：简繁转换在 get_task_result read-time 按当前 output_script 做（动态切繁简）
    let output_script = crate::models::config::OutputScript::Original;

    // 模型解析 + ensure + 构造分派产物（按 backend 分派）。两库引擎统一为 SttInvocation。
    let invocation: crate::stt::SttInvocation = match backend {
        crate::models::config::SttBackend::Whisper => {
            // 模型 id：task 透传优先，回退 config.whisper.default_model
            let default_model = ctx.model_service.default_model();
            let model = task.model.as_deref().unwrap_or(default_model).to_string();
            // ensure_model：模型缺失时自动下载（接入 HTTP，修复旧版 auto_download 形同虚设）
            ctx.model_service
                .ensure_model(&model)
                .await
                .map_err(|e| -> Error {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                        format!("模型准备失败: {e}"),
                    ))))
                })?;
            let model_path = ctx
                .model_service
                .get_model_path(&model)
                .map_err(|e| -> Error {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                        format!("获取模型路径失败: {e}"),
                    ))))
                })?;
            crate::stt::SttInvocation::TranscribeRs {
                model_id: model,
                spec: crate::stt::SttEngineSpec::Whisper { model_path },
            }
        }
        crate::models::config::SttBackend::SenseVoice => {
            // SenseVoice：从 config.whisper.engine.sensevoice 解析目录 + 量化（host 预置，无自动下载）
            #[cfg(feature = "sensevoice")]
            {
                let models_dir = cfg.whisper.models_dir.clone();
                let sv_cfg = &cfg.whisper.engine.sensevoice;
                let model_dir = crate::stt::sensevoice::resolve_model_dir(&models_dir, sv_cfg);
                let quantization = crate::stt::sensevoice::parse_quantization(&sv_cfg.quantization)
                    .map_err(|e| -> Error {
                        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
                            format!("SenseVoice 量化解析失败: {e}"),
                        ))))
                    })?;
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
                return Err(Error::Abort(std::sync::Arc::new(Box::new(
                    std::io::Error::other(
                        "backend=sensevoice 但未启用 sensevoice feature（请 --features sensevoice 编译）",
                    ),
                ))));
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
            let sh = &cfg.whisper.engine.sherpa;
            let models_dir = std::path::Path::new(&cfg.whisper.models_dir);
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

    // 同步推理走 spawn_blocking（两库引擎都是同步阻塞 C 调用，不能阻塞 tokio reactor）。
    // 修复旧版反模式：transcribe_with_conversion 内 spawn_blocking 再 block_on 新 current-thread runtime。
    // 无音频流时 ffmpeg-sidecar 转码自然报错 → SttError::Audio（替代旧 ffprobe 系统依赖）。
    let audio_path_for_blocking = task.processed_audio_path.clone();
    // STT 参数（config 默认已在提交时合并进 task.language/initial_prompt）
    let opt_language = task.language.clone();
    let opt_initial_prompt = task.initial_prompt.clone();
    let mut response = tokio::task::spawn_blocking(
        move || -> std::result::Result<crate::models::request::TranscriptionResponse, crate::stt::SttError>
    {
            let samples = crate::stt::audio::to_whisper_samples(&audio_path_for_blocking)?;
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
                    // 共享 trait 方法（whisper + sensevoice 都实现 SpeechModel::transcribe）
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
    .map_err(|e| {
        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
            format!("转录任务 join 失败: {e}"),
        ))))
    })?
    .map_err(|e| {
        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::other(
            format!("转录失败: {}", e),
        ))))
    })?;

    // 设置元数据和时长
    if let Some(meta) = &metadata {
        response.duration = Some(meta.duration_seconds as f32);
        response.metadata = Some(meta.clone());
    }

    let completed_task = TranscriptionCompletedTask {
        task_id: task.task_id.clone(),
        transcription_result: response,
        response_format: task.response_format,
        metadata,
        created_at: task.created_at,
    };

    info!(
        "Whisper transcription completed: {} ({} characters)",
        task.task_id,
        completed_task.transcription_result.text.len()
    );
    Ok(completed_task)
}

/// 步骤 3: 结果格式化和存储
async fn result_formatting_step(
    task: TranscriptionCompletedTask,
    ctx: Data<StepContext>,
) -> Result<(), Error> {
    info!("Step 3 - Result Formatting and Storage: {}", task.task_id);

    // 保存结果到SQLite存储（包含元数据）
    ctx.save_task_result(&task.task_id, &task.transcription_result, &task.metadata)
        .await?;

    // 计算实际处理时间
    let now = Utc::now();
    let processing_time = now.signed_duration_since(task.created_at);
    let duration = Duration::from_secs(processing_time.num_seconds().max(0) as u64);

    // 更新状态为完成
    let result_summary = if let Some(metadata) = &task.metadata {
        format!(
            "转录了 {} 个字符，文件: {} ({:.2}s)",
            task.transcription_result.text.len(),
            metadata.format,
            metadata.duration_seconds
        )
    } else {
        format!("转录了 {} 个字符", task.transcription_result.text.len())
    };

    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Completed {
            completed_at: now,
            processing_time: duration,
            result_summary: Some(result_summary),
        },
    )
    .await?;

    info!(
        "Transcription task completed: {} (processing time: {}s)",
        task.task_id,
        duration.as_secs()
    );
    Ok(())
}

/// 转录流水线 worker - 内部调用步骤函数
pub async fn transcription_pipeline_worker(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<(), Error> {
    info!(
        "Start processing the transcription pipeline: {}",
        task.task_id
    );

    // 步骤 1: 音频预处理
    let audio_processed_task = match audio_preprocessing_step(task.clone(), ctx.clone()).await {
        Ok(task) => task,
        Err(e) => {
            let error_msg = format!("音频预处理失败: {}", e);
            warn!("Step 1 failed: {} - {}", task.task_id, error_msg);

            // 更新任务状态为失败
            if let Err(save_err) = ctx
                .save_task_status(
                    &task.task_id,
                    &TaskStatus::Failed {
                        error: TaskError::AudioProcessingFailed {
                            stage: ProcessingStage::AudioFormatDetection,
                            message: error_msg.clone(),
                            is_recoverable: true,
                        },
                        failed_at: Utc::now(),
                        retry_count: 0,
                        is_recoverable: true,
                    },
                )
                .await
            {
                warn!(
                    "Failed to save failed status: {} - {}",
                    task.task_id, save_err
                );
            }

            return Err(e);
        }
    };

    // 步骤 2: Whisper 转录
    let transcription_completed_task =
        match transcription_step(audio_processed_task, ctx.clone()).await {
            Ok(task) => task,
            Err(e) => {
                let error_msg = format!("Whisper转录失败: {}", e);
                warn!("Step 2 failed: {} - {}", task.task_id, error_msg);

                // 更新任务状态为失败
                if let Err(save_err) = ctx
                    .save_task_status(
                        &task.task_id,
                        &TaskStatus::Failed {
                            error: TaskError::TranscriptionFailed {
                                model: task.model.clone().unwrap_or_else(|| "unknown".to_string()),
                                message: error_msg.clone(),
                                is_recoverable: true,
                            },
                            failed_at: Utc::now(),
                            retry_count: 0,
                            is_recoverable: true,
                        },
                    )
                    .await
                {
                    warn!(
                        "Failed to save failed status: {} - {}",
                        task.task_id, save_err
                    );
                }

                return Err(e);
            }
        };

    // 步骤 3: 结果格式化和存储
    if let Err(e) = result_formatting_step(transcription_completed_task, ctx.clone()).await {
        let error_msg = format!("结果格式化失败: {}", e);
        warn!("Step 3 failed: {} - {}", task.task_id, error_msg);

        // 更新任务状态为失败
        if let Err(save_err) = ctx
            .save_task_status(
                &task.task_id,
                &TaskStatus::Failed {
                    error: TaskError::StorageError {
                        operation: "result_formatting".to_string(),
                        message: error_msg.clone(),
                    },
                    failed_at: Utc::now(),
                    retry_count: 0,
                    is_recoverable: true,
                },
            )
            .await
        {
            warn!(
                "Failed to save failed status: {} - {}",
                task.task_id, save_err
            );
        }

        return Err(e);
    }

    info!("Transcription pipeline completed: {}", task.task_id);
    Ok(())
}
