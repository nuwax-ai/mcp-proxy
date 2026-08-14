//! 任务队列端点：状态/结果查询、取消、重试、删除与统计（STT + TTS）。

use super::*;

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
