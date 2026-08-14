//! STT 请求解析共用件：multipart 流式解析（sync/async 双 handler 共用）、
//! URL 文件名提取与请求类型定义。

use super::*;

/// 转录请求数据
#[derive(Debug)]
pub(super) struct TranscriptionRequest {
    pub(super) filename: String,
    pub(super) model: Option<String>,
    pub(super) response_format: Option<String>,
    /// 目标语种（BCP-47，如 `"en"`/`"zh"`；`None` = 自动检测）
    pub(super) language: Option<String>,
    /// 初始提示，给模型领域上下文（提升专有词 / 风格准确率）
    pub(super) initial_prompt: Option<String>,
    /// STT 后端覆盖：`whisper` / `sensevoice`（仅同步 /transcribe 生效，用于 A/B 对比；
    /// `None` = 用 config.whisper.engine.backend）。异步任务忽略此字段（config-only）。
    pub(super) engine: Option<crate::models::config::SttBackend>,
}

/// URL转录请求数据
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UrlTranscriptionRequest {
    pub(super) url: String,
    pub(super) model: Option<String>,
    pub(super) response_format: Option<String>,
    /// 目标语种（BCP-47，如 `"en"`/`"zh"`；`None` = 自动检测）
    pub(super) language: Option<String>,
    /// 初始提示，给模型领域上下文（提升专有词 / 风格准确率）
    pub(super) initial_prompt: Option<String>,
}

/// 解析 multipart 请求，使用流式处理避免内存占用
pub(super) async fn extract_transcription_request_streaming(
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
pub(super) fn extract_filename_from_url(url: &str) -> Option<String> {
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
