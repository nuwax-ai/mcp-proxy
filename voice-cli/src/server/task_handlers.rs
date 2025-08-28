use crate::models::{
    AsyncTaskResponse, CancelResponse, TaskStatusResponse, TranscriptionResponse, TaskStatus
};
use crate::services::ApalisManager;
use crate::VoiceCliError;

use axum::{
    extract::{Multipart, Path, State},
    response::Json,
};
use bytes::Bytes;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use utoipa;

/// 应用状态
#[derive(Clone)]
pub struct TaskAppState {
    pub apalis_manager: Arc<ApalisManager>,
}

/// 提交异步转录任务
/// POST /tasks/transcribe
#[utoipa::path(
    post,
    path = "/tasks/transcribe",
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
pub async fn submit_transcription_task(
    State(state): State<TaskAppState>,
    multipart: Multipart,
) -> Result<Json<AsyncTaskResponse>, VoiceCliError> {
    let task_id = generate_task_id();
    info!("开始处理异步转录请求: {}", task_id);

    // 1. 解析 multipart 数据
    let (audio_data, request) = extract_transcription_request(multipart).await?;

    // 2. 验证音频文件
    validate_audio_file(&audio_data, &request.filename)?;

    // 3. 保存音频文件
    let audio_file_manager = crate::services::AudioFileManager::new("./data/audio")
        .map_err(|e| VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))?;
    
    let audio_file_path = audio_file_manager.save_audio_file(&task_id, &audio_data, &request.filename).await
        .map_err(|e| VoiceCliError::Storage(format!("保存音频文件失败: {}", e)))?;

    // 4. 提交任务到队列
    let returned_task_id = state.apalis_manager.submit_task(
        audio_file_path,
        request.filename,
        request.model,
        request.response_format,
    ).await?;

    info!("异步转录任务提交成功: {}", returned_task_id);
    
    let response = AsyncTaskResponse {
        task_id: returned_task_id,
        status: TaskStatus::Pending {
            queued_at: chrono::Utc::now(),
        },
        estimated_completion: None,
    };
    
    Ok(Json(response))
}

/// 获取任务状态
/// GET /tasks/{task_id}
#[utoipa::path(
    get,
    path = "/tasks/{task_id}",
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
pub async fn get_task_status(
    State(state): State<TaskAppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TaskStatusResponse>, VoiceCliError> {
    match state.apalis_manager.get_task_status(&task_id).await? {
        Some(status) => {
            info!("获取任务状态成功: {} -> {:?}", task_id, status);
            let response = TaskStatusResponse {
                task_id: task_id.clone(),
                status,
                created_at: chrono::Utc::now(), // 简化版本，实际应从数据库获取
                updated_at: chrono::Utc::now(),
            };
            Ok(Json(response))
        }
        None => {
            warn!("任务不存在: {}", task_id);
            Err(VoiceCliError::NotFound(format!("任务 '{}' 不存在", task_id)))
        }
    }
}

/// 获取任务结果
/// GET /tasks/{task_id}/result
#[utoipa::path(
    get,
    path = "/tasks/{task_id}/result",
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
pub async fn get_task_result(
    State(state): State<TaskAppState>,
    Path(task_id): Path<String>,
) -> Result<Json<TranscriptionResponse>, VoiceCliError> {
    match state.apalis_manager.get_task_result(&task_id).await? {
        Some(result) => {
            info!("获取任务结果成功: {} -> {} 字符", task_id, result.text.len());
            Ok(Json(result))
        }
        None => {
            warn!("任务结果不可用: {}", task_id);
            Err(VoiceCliError::NotFound(format!("任务 '{}' 的结果不可用", task_id)))
        }
    }
}

/// 取消任务
/// DELETE /tasks/{task_id}
#[utoipa::path(
    delete,
    path = "/tasks/{task_id}",
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
pub async fn cancel_task(
    State(state): State<TaskAppState>,
    Path(task_id): Path<String>,
) -> Result<Json<CancelResponse>, VoiceCliError> {
    let cancelled = state.apalis_manager.cancel_task(&task_id).await?;
    
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
    Ok(Json(response))
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
                let data = field.bytes().await
                    .map_err(|e| VoiceCliError::MultipartError(format!("读取音频数据失败: {}", e)))?;
                audio_data = Some(data);
                info!("接收音频文件: {} bytes, 文件名: {:?}", 
                      audio_data.as_ref().unwrap().len(), filename);
            }
            "model" => {
                model = Some(field.text().await
                    .map_err(|e| VoiceCliError::MultipartError(format!("解析模型参数失败: {}", e)))?);
            }
            "response_format" => {
                response_format = Some(field.text().await
                    .map_err(|e| VoiceCliError::MultipartError(format!("解析响应格式参数失败: {}", e)))?);
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

    info!("音频文件验证通过: {} ({} bytes)", filename, audio_data.len());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_task_id() {
        let task_id = generate_task_id();
        assert!(task_id.starts_with("task_"));
        assert!(task_id.len() > 10);
    }

    #[test]
    fn test_validate_audio_file() {
        let small_data = Bytes::from(vec![0u8; 1024]);
        assert!(validate_audio_file(&small_data, "test.wav").is_ok());

        let large_data = Bytes::from(vec![0u8; 300 * 1024 * 1024]); // 300MB
        assert!(validate_audio_file(&large_data, "large.wav").is_err());
    }
}