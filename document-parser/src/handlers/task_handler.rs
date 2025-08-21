use crate::error::AppError;
use crate::handlers::response::{
    ApiResponse, BatchOperationResponse, TaskOperationResponse,
};
use crate::handlers::validation::RequestValidator;
use crate::models::{
    DocumentFormat, DocumentTask, HttpResult, ParserEngine, SourceType, TaskStatus,
};
use crate::services::TaskStats;
use crate::{app_state::AppState, handlers::SimpleTaskStatus};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{error, info, warn};
use utoipa::ToSchema;

/// 创建任务请求
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateTaskRequest {
    pub source_type: SourceType,
    pub source_path: Option<String>,
    pub format: DocumentFormat,
}

/// 任务查询参数
#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct TaskQueryParams {
    /// 页码，从1开始
    pub page: Option<usize>,
    /// 每页大小
    pub page_size: Option<usize>,
    /// 任务状态过滤
    pub status: Option<TaskStatus>,
    /// 文档格式过滤
    pub format: Option<DocumentFormat>,
    /// 源类型过滤
    pub source_type: Option<SourceType>,
    /// 排序字段
    pub sort_by: Option<String>,
    /// 排序方向
    pub sort_order: Option<String>,
    /// 搜索关键词
    pub search: Option<String>,
    /// 创建时间范围过滤 - 开始时间 (ISO 8601)
    pub created_after: Option<String>,
    /// 创建时间范围过滤 - 结束时间 (ISO 8601)
    pub created_before: Option<String>,
    /// 文件大小范围过滤 - 最小大小（字节）
    pub min_file_size: Option<u64>,
    /// 文件大小范围过滤 - 最大大小（字节）
    pub max_file_size: Option<u64>,
}

/// 批量操作请求
#[derive(Debug, Serialize, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct BatchOperationRequest {
    /// 任务ID列表
    pub task_ids: Vec<String>,
    /// 操作类型
    pub operation: BatchOperation,
    /// 操作原因（可选）
    pub reason: Option<String>,
}

/// 批量操作类型
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchOperation {
    Cancel,
    Delete,
    Retry,
}

/// 任务过滤器
#[derive(Debug, Clone)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub format: Option<DocumentFormat>,
    pub source_type: Option<SourceType>,
    pub search: Option<String>,
    pub created_after: Option<chrono::DateTime<chrono::Utc>>,
    pub created_before: Option<chrono::DateTime<chrono::Utc>>,
    pub min_file_size: Option<u64>,
    pub max_file_size: Option<u64>,
}

/// 取消任务请求
#[derive(Debug, Serialize, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct CancelTaskRequest {
    pub reason: Option<String>,
}

/// 任务响应
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskResponse {
    pub task: DocumentTask,
}

/// 任务列表响应
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskListResponse {
    pub tasks: Vec<TaskSummary>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
}

/// 任务摘要（避免循环引用）
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskSummary {
    pub id: String,
    pub status: TaskStatus,
    pub source_type: SourceType,
    pub source_path: Option<String>,
    pub source_url: Option<String>,
    pub document_format: Option<DocumentFormat>,
    pub parser_engine: Option<ParserEngine>,
    pub backend: String,
    pub progress: u32,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: String,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
}

impl From<DocumentTask> for TaskSummary {
    fn from(task: DocumentTask) -> Self {
        Self {
            id: task.id,
            status: task.status,
            source_type: task.source_type,
            source_path: task.source_path,
            source_url: task.source_url,
            document_format: task.document_format,
            parser_engine: task.parser_engine,
            backend: task.backend,
            progress: task.progress,
            error_message: task.error_message,
            created_at: task.created_at.to_rfc3339(),
            updated_at: task.updated_at.to_rfc3339(),
            expires_at: task.expires_at.to_rfc3339(),
            file_size: task.file_size,
            mime_type: task.mime_type,
            retry_count: task.retry_count,
            max_retries: task.max_retries,
        }
    }
}

/// 任务统计响应
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaskStatsResponse {
    pub stats: TaskStats,
}

/// 创建任务
#[utoipa::path(
    post,
    path = "/api/v1/tasks",
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "任务创建成功", body = TaskOperationResponse),
        (status = 400, description = "请求参数错误", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> impl axum::response::IntoResponse {
    info!("创建任务请求: {:?}", request);

    // 验证文档格式
    if let Err(e) = RequestValidator::validate_document_format(&request.format) {
        return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
    }

    // 验证源路径（如果提供）
    if let Some(ref source_path) = request.source_path {
        match request.source_type {
            SourceType::Url => {
                if let Err(e) = RequestValidator::validate_url(source_path) {
                    return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
                }
            }
            _ => {} // Upload类型的路径在上传时验证
        }
    }

    // 创建任务
    match state
        .task_service
        .create_task(
            request.source_type,
            request.source_path,
            None, // 通过API创建任务时暂时不设置原始文件名
            None,
        )
        .await
    {
        Ok(task) => {
            info!("任务创建成功: {}", task.id);
            let complete = task.status.is_terminal();
            let response = TaskOperationResponse {
                task_id: task.id.clone(),
                operation: "create".to_string(),
                message: "任务创建成功".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                // task: Some(task), // 移除以避免循环引用
                complete,
                status: SimpleTaskStatus::from(&task.status),
            };
            ApiResponse::success_with_status(response, StatusCode::CREATED).into_response()
        }
        Err(e) => {
            error!("任务创建失败: {}", e);
            ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response()
        }
    }
}

/// 获取任务详情
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "获取任务详情成功", body = TaskOperationResponse),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl axum::response::IntoResponse {
    info!("获取任务详情请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
    }

    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            info!("获取任务详情成功: {}", task_id);
            let complete = task.status.is_terminal();

            // 如果任务状态为 Failed，使用 ApiResponse::error 返回错误响应
            if task.status.is_failed() {
                if let Some(error) = task.status.get_error() {
                    return ApiResponse::error::<TaskOperationResponse>(
                        error.error_code.clone(),
                        error.error_message.clone(),
                    )
                    .into_response();
                }
            }

            let response = TaskOperationResponse {
                task_id: task.id.clone(),
                operation: "get".to_string(),
                message: "获取任务详情成功".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                // task: Some(task), // 移除以避免循环引用
                complete,
                status: SimpleTaskStatus::from(&task.status),
            };
            ApiResponse::success(response).into_response()
        }
        Ok(None) => {
            warn!("任务不存在: {}", task_id);
            ApiResponse::not_found::<TaskOperationResponse>(&format!("任务不存在: {task_id}"))
                .into_response()
        }
        Err(e) => {
            error!("获取任务详情失败: task_id={}, error={}", task_id, e);
            ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response()
        }
    }
}

/// 获取任务列表
#[utoipa::path(
    get,
    path = "/api/v1/tasks",
    params(TaskQueryParams),
    responses(
        (status = 200, description = "获取任务列表成功", body = TaskListResponse),
        (status = 400, description = "请求参数错误", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn list_tasks(
    State(state): State<AppState>,
    Query(params): Query<TaskQueryParams>,
) -> impl axum::response::IntoResponse {
    info!("获取任务列表请求: {:?}", params);

    // 验证分页参数
    let (page, page_size) =
        match RequestValidator::validate_pagination(params.page, params.page_size) {
            Ok(result) => result,
            Err(e) => {
                return ApiResponse::from_app_error::<TaskListResponse>(e).into_response();
            }
        };

    // 验证排序参数
    let (sort_by, sort_order) = match RequestValidator::validate_sort_params(
        params.sort_by.as_deref(),
        params.sort_order.as_deref(),
    ) {
        Ok(result) => result,
        Err(e) => {
            return ApiResponse::from_app_error::<TaskListResponse>(e).into_response();
        }
    };

    // 构建过滤器
    let filter = match build_task_filter(&params) {
        Ok(filter) => filter,
        Err(e) => {
            return ApiResponse::from_app_error::<TaskListResponse>(e).into_response();
        }
    };

    // 获取所有任务并应用过滤和排序
    match state.task_service.list_tasks(None).await {
        Ok(all_tasks) => {
            // 应用过滤器
            let filtered_tasks: Vec<DocumentTask> = all_tasks
                .into_iter()
                .filter(|task| apply_task_filter(task, &filter))
                .collect();

            // 应用排序
            let mut sorted_tasks = filtered_tasks;
            apply_task_sorting(&mut sorted_tasks, &sort_by, &sort_order);

            let total = sorted_tasks.len();
            let start = (page - 1) * page_size;
            let end = std::cmp::min(start + page_size, total);
            let tasks = if start < total {
                sorted_tasks[start..end].to_vec()
            } else {
                Vec::new()
            };

            info!("获取任务列表成功: {} 个任务，第 {} 页", total, page);

            let total_pages = total.div_ceil(page_size);
            let task_summaries: Vec<TaskSummary> =
                tasks.into_iter().map(TaskSummary::from).collect();
            let response = TaskListResponse {
                tasks: task_summaries,
                total,
                page,
                page_size,
                total_pages,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("获取任务列表失败: {}", e);
            ApiResponse::from_app_error::<TaskListResponse>(e).into_response()
        }
    }
}

/// 取消任务
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/cancel",
    params(
        ("task_id" = String, Path, description = "任务ID"),
        ("reason" = Option<String>, Query, description = "取消原因")
    ),
    responses(
        (status = 200, description = "任务取消成功", body = TaskOperationResponse),
        (status = 400, description = "任务无法取消", body = HttpResult<String>),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn cancel_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Query(request): Query<CancelTaskRequest>,
) -> impl axum::response::IntoResponse {
    info!("取消任务请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
    }

    // 检查任务是否存在和状态
    match state.task_service.get_task(&task_id).await {
        Ok(Some(existing_task)) => {
            if !can_cancel_task(&existing_task.status) {
                let error =
                    AppError::Task(format!("任务状态为 {:?}，无法取消", existing_task.status));
                return ApiResponse::from_app_error::<TaskOperationResponse>(error).into_response();
            }
        }
        Ok(None) => {
            return ApiResponse::not_found::<TaskOperationResponse>(&format!(
                "任务不存在: {task_id}"
            ))
            .into_response();
        }
        Err(e) => {
            error!("检查任务状态失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
        }
    }

    // 执行取消操作
    match state
        .task_service
        .cancel_task(&task_id, request.reason.map(|s| s.to_string()))
        .await
    {
        Ok(task) => {
            info!("任务取消成功: {}", task_id);
            let complete = task.status.is_terminal();
            let response = TaskOperationResponse {
                task_id: task.id.clone(),
                operation: "cancel".to_string(),
                message: "任务取消成功".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                // task: Some(task), // 移除以避免循环引用
                complete,
                status: SimpleTaskStatus::from(&task.status),
            };
            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("任务取消失败: task_id={}, error={}", task_id, e);
            ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response()
        }
    }
}

/// 删除任务
#[utoipa::path(
    delete,
    path = "/api/v1/tasks/{task_id}",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "任务删除成功", body = TaskOperationResponse),
        (status = 400, description = "任务无法删除", body = HttpResult<String>),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn delete_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl axum::response::IntoResponse {
    info!("删除任务请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
    }

    // 检查任务是否存在和状态
    match state.task_service.get_task(&task_id).await {
        Ok(Some(existing_task)) => {
            if !can_delete_task(&existing_task.status) {
                let error =
                    AppError::Task(format!("任务状态为 {:?}，无法删除", existing_task.status));
                return ApiResponse::from_app_error::<TaskOperationResponse>(error).into_response();
            }
        }
        Ok(None) => {
            return ApiResponse::not_found::<TaskOperationResponse>(&format!(
                "任务不存在: {task_id}"
            ))
            .into_response();
        }
        Err(e) => {
            error!("检查任务状态失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
        }
    }

    // 执行删除操作
    match state.task_service.delete_task(&task_id).await {
        Ok(_) => {
            info!("任务删除成功: {}", task_id);
            let response = TaskOperationResponse {
                task_id: task_id.clone(),
                operation: "delete".to_string(),
                message: "任务删除成功".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                // task: None, // 移除以避免循环引用
                complete: true,                      // 删除操作本身就是完成的
                status: SimpleTaskStatus::Completed, // 删除操作完成
            };
            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("任务删除失败: task_id={}, error={}", task_id, e);
            ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response()
        }
    }
}

/// 批量操作任务
#[utoipa::path(
    post,
    path = "/api/v1/tasks/batch",
    params(
        ("task_ids" = Vec<String>, Query, description = "任务ID列表"),
        ("operation" = BatchOperation, Query, description = "操作类型"),
        ("reason" = Option<String>, Query, description = "操作原因")
    ),
    responses(
        (status = 200, description = "批量操作成功", body = BatchOperationResponse),
        (status = 400, description = "请求参数错误", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn batch_operation_tasks(
    State(state): State<AppState>,
    Query(request): Query<BatchOperationRequest>,
) -> impl axum::response::IntoResponse {
    info!("批量操作任务请求: {:?}", request.operation);

    let total = request.task_ids.len();
    let mut successful = 0;
    let mut failed = 0;
    let mut errors = Vec::new();

    for task_id in &request.task_ids {
        // 验证任务ID
        if let Err(e) = RequestValidator::validate_task_id(task_id) {
            failed += 1;
            errors.push(crate::handlers::response::BatchError {
                item_id: task_id.clone(),
                error_code: e.get_error_code().to_string(),
                error_message: e.to_string(),
            });
            continue;
        }

        let result = match request.operation {
            BatchOperation::Cancel => state
                .task_service
                .cancel_task(task_id, request.reason.clone())
                .await
                .map(|_| ()),
            BatchOperation::Delete => state.task_service.delete_task(task_id).await.map(|_| ()),
            BatchOperation::Retry => state.task_service.retry_task(task_id).await.map(|_| ()),
        };

        match result {
            Ok(_) => {
                successful += 1;
                info!(
                    "批量操作成功: task_id={}, operation={:?}",
                    task_id, request.operation
                );
            }
            Err(e) => {
                failed += 1;
                error!(
                    "批量操作失败: task_id={}, operation={:?}, error={}",
                    task_id, request.operation, e
                );
                errors.push(crate::handlers::response::BatchError {
                    item_id: task_id.clone(),
                    error_code: e.get_error_code().to_string(),
                    error_message: e.to_string(),
                });
            }
        }
    }

    let response = BatchOperationResponse {
        total,
        successful,
        failed,
        errors,
    };

    info!(
        "批量操作完成: 总计={}, 成功={}, 失败={}",
        total, successful, failed
    );

    if failed == 0 {
        ApiResponse::success(response).into_response()
    } else if successful == 0 {
        ApiResponse::error_with_status::<BatchOperationResponse>(
            "BATCH_OPERATION_ALL_FAILED".to_string(),
            "所有操作都失败了".to_string(),
            StatusCode::BAD_REQUEST,
        )
        .into_response()
    } else {
        // 部分成功
        ApiResponse::success_with_status(response, StatusCode::PARTIAL_CONTENT).into_response()
    }
}

/// 重试任务
#[utoipa::path(
    post,
    path = "/api/v1/tasks/{task_id}/retry",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "任务重试成功", body = TaskOperationResponse),
        (status = 400, description = "任务无法重试", body = HttpResult<String>),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn retry_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl axum::response::IntoResponse {
    info!("重试任务请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
    }

    // 检查任务是否存在和状态
    match state.task_service.get_task(&task_id).await {
        Ok(Some(existing_task)) => {
            if !can_retry_task(&existing_task.status) {
                let error =
                    AppError::Task(format!("任务状态为 {:?}，无法重试", existing_task.status));
                return ApiResponse::from_app_error::<TaskOperationResponse>(error).into_response();
            }
        }
        Ok(None) => {
            return ApiResponse::not_found::<TaskOperationResponse>(&format!(
                "任务不存在: {task_id}"
            ))
            .into_response();
        }
        Err(e) => {
            error!("检查任务状态失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response();
        }
    }

    // 执行重试操作
    match state.task_service.retry_task(&task_id).await {
        Ok(task) => {
            info!("任务重试成功: {}", task_id);
            let complete = task.status.is_terminal();
            let response = TaskOperationResponse {
                task_id: task.id.clone(),
                operation: "retry".to_string(),
                message: "任务重试成功".to_string(),
                timestamp: chrono::Utc::now().to_rfc3339(),
                // task: Some(task), // 移除以避免循环引用
                complete,
                status: SimpleTaskStatus::from(&task.status),
            };
            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("任务重试失败: task_id={}, error={}", task_id, e);
            ApiResponse::from_app_error::<TaskOperationResponse>(e).into_response()
        }
    }
}

/// 获取任务统计
#[utoipa::path(
    get,
    path = "/api/v1/tasks/stats",
    responses(
        (status = 200, description = "获取任务统计成功", body = TaskStatsResponse),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn get_task_stats(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    info!("获取任务统计请求");

    match state.task_service.get_task_stats().await {
        Ok(stats) => {
            info!("获取任务统计成功");
            ApiResponse::success(TaskStatsResponse { stats }).into_response()
        }
        Err(e) => {
            error!("获取任务统计失败: {}", e);
            ApiResponse::from_app_error::<TaskStatsResponse>(e).into_response()
        }
    }
}

/// 清理过期任务
#[utoipa::path(
    post,
    path = "/api/v1/tasks/cleanup",
    responses(
        (status = 200, description = "清理过期任务成功", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn cleanup_expired_tasks(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    info!("清理过期任务请求");

    match state.task_service.cleanup_expired_tasks().await {
        Ok(count) => {
            info!("清理过期任务完成，删除了 {} 个任务", count);
            ApiResponse::message(format!("清理过期任务完成，删除了 {count} 个任务"))
                .into_response()
        }
        Err(e) => {
            error!("清理过期任务失败: {}", e);
            ApiResponse::from_app_error::<String>(e).into_response()
        }
    }
}

/// 获取任务进度
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/progress",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "获取任务进度成功", body = TaskOperationResponse),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn get_task_progress(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl axum::response::IntoResponse {
    info!("获取任务进度请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<HashMap<String, serde_json::Value>>(e)
            .into_response();
    }

    // 由于 get_task_progress 方法不存在，我们暂时返回任务信息
    match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => {
            info!("获取任务进度成功: {}", task_id);
            let mut progress = HashMap::new();
            progress.insert("task_id".to_string(), serde_json::Value::String(task.id));
            progress.insert(
                "status".to_string(),
                serde_json::Value::String(format!("{:?}", task.status)),
            );
            progress.insert(
                "created_at".to_string(),
                serde_json::Value::String(task.created_at.to_rfc3339()),
            );
            progress.insert(
                "updated_at".to_string(),
                serde_json::Value::String(task.updated_at.to_rfc3339()),
            );
            ApiResponse::success(progress).into_response()
        }
        Ok(None) => ApiResponse::not_found::<HashMap<String, serde_json::Value>>(&format!(
            "任务不存在: {task_id}"
        ))
        .into_response(),
        Err(e) => {
            error!("获取任务进度失败: task_id={}, error={}", task_id, e);
            ApiResponse::from_app_error::<HashMap<String, serde_json::Value>>(e).into_response()
        }
    }
}

// 辅助函数

/// 构建任务过滤器
fn build_task_filter(params: &TaskQueryParams) -> Result<TaskFilter, AppError> {
    let mut filter = TaskFilter {
        status: params.status.clone(),
        format: params.format.clone(),
        source_type: params.source_type.clone(),
        search: params.search.clone(),
        created_after: None,
        created_before: None,
        min_file_size: params.min_file_size,
        max_file_size: params.max_file_size,
    };

    // 解析时间范围
    if let Some(ref created_after) = params.created_after {
        filter.created_after = Some(
            chrono::DateTime::parse_from_rfc3339(created_after)
                .map_err(|e| AppError::Validation(format!("无效的开始时间格式: {e}")))?
                .with_timezone(&chrono::Utc),
        );
    }

    if let Some(ref created_before) = params.created_before {
        filter.created_before = Some(
            chrono::DateTime::parse_from_rfc3339(created_before)
                .map_err(|e| AppError::Validation(format!("无效的结束时间格式: {e}")))?
                .with_timezone(&chrono::Utc),
        );
    }

    // 验证时间范围
    if let (Some(after), Some(before)) = (filter.created_after, filter.created_before) {
        if after >= before {
            return Err(AppError::Validation("开始时间必须早于结束时间".to_string()));
        }
    }

    // 验证文件大小范围
    if let (Some(min_size), Some(max_size)) = (filter.min_file_size, filter.max_file_size) {
        if min_size >= max_size {
            return Err(AppError::Validation(
                "最小文件大小必须小于最大文件大小".to_string(),
            ));
        }
    }

    Ok(filter)
}

/// 检查任务是否可以取消
fn can_cancel_task(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Pending { .. } | TaskStatus::Processing { .. }
    )
}

/// 检查任务是否可以删除
fn can_delete_task(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Completed { .. } | TaskStatus::Failed { .. } | TaskStatus::Cancelled { .. }
    )
}

/// 检查任务是否可以重试
fn can_retry_task(status: &TaskStatus) -> bool {
    matches!(status, TaskStatus::Failed { .. })
}

/// 应用任务过滤器
fn apply_task_filter(task: &DocumentTask, filter: &TaskFilter) -> bool {
    // 状态过滤
    if let Some(ref status) = filter.status {
        if &task.status != status {
            return false;
        }
    }

    // 格式过滤
    if let Some(ref format) = filter.format {
        if task.document_format.as_ref() != Some(format) {
            return false;
        }
    }

    // 源类型过滤
    if let Some(ref source_type) = filter.source_type {
        if &task.source_type != source_type {
            return false;
        }
    }

    // 搜索关键词过滤
    if let Some(ref search) = filter.search {
        let search_lower = search.to_lowercase();
        let matches_id = task.id.to_lowercase().contains(&search_lower);
        let matches_path = task
            .source_path
            .as_ref()
            .map(|p| p.to_lowercase().contains(&search_lower))
            .unwrap_or(false);
        let matches_url = task
            .source_url
            .as_ref()
            .map(|u| u.to_lowercase().contains(&search_lower))
            .unwrap_or(false);
        if !matches_id && !matches_path && !matches_url {
            return false;
        }
    }

    // 创建时间范围过滤
    if let Some(created_after) = filter.created_after {
        if task.created_at <= created_after {
            return false;
        }
    }

    if let Some(created_before) = filter.created_before {
        if task.created_at >= created_before {
            return false;
        }
    }

    // 文件大小范围过滤（这里假设 DocumentTask 有 file_size 字段，如果没有则忽略）
    // if let Some(min_size) = filter.min_file_size {
    //     if task.file_size.unwrap_or(0) < min_size {
    //         return false;
    //     }
    // }

    // if let Some(max_size) = filter.max_file_size {
    //     if task.file_size.unwrap_or(0) > max_size {
    //         return false;
    //     }
    // }

    true
}

/// 应用任务排序
fn apply_task_sorting(tasks: &mut Vec<DocumentTask>, sort_by: &str, sort_order: &str) {
    let ascending = sort_order == "asc";

    match sort_by {
        "created_at" => {
            if ascending {
                tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
            } else {
                tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
        }
        "updated_at" => {
            if ascending {
                tasks.sort_by(|a, b| a.updated_at.cmp(&b.updated_at));
            } else {
                tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            }
        }
        "status" => {
            if ascending {
                tasks.sort_by(|a, b| format!("{:?}", a.status).cmp(&format!("{:?}", b.status)));
            } else {
                tasks.sort_by(|a, b| format!("{:?}", b.status).cmp(&format!("{:?}", a.status)));
            }
        }
        _ => {
            // 默认按创建时间降序排序
            tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        }
    }
}

/// 任务结果概览响应
#[derive(Debug, Serialize, ToSchema)]
pub struct TaskResultSummaryResponse {
    pub task_id: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub updated_at: String,
    pub file_info: Option<TaskFileInfo>,
    pub oss_info: Option<TaskOssInfo>,
    pub processing_stats: Option<TaskProcessingStats>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskFileInfo {
    pub original_filename: Option<String>,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
    pub format: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskOssInfo {
    pub bucket: String,
    pub markdown_available: bool,
    pub images_count: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaskProcessingStats {
    pub processing_time: Option<String>,
    pub word_count: Option<usize>,
    pub page_count: Option<usize>,
}

/// 获取任务结果概览
#[utoipa::path(
    get,
    path = "/api/v1/tasks/{task_id}/result",
    params(
        ("task_id" = String, Path, description = "任务ID")
    ),
    responses(
        (status = 200, description = "获取任务结果成功", body = TaskResultSummaryResponse),
        (status = 404, description = "任务不存在", body = HttpResult<String>),
        (status = 500, description = "服务器内部错误", body = HttpResult<String>)
    ),
    tag = "任务管理"
)]
pub async fn get_task_result(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> impl axum::response::IntoResponse {
    info!("获取任务结果概览请求: {}", task_id);

    // 验证任务ID
    if let Err(e) = RequestValidator::validate_task_id(&task_id) {
        return ApiResponse::from_app_error::<TaskResultSummaryResponse>(e).into_response();
    }

    // 获取任务详情
    let task = match state.task_service.get_task(&task_id).await {
        Ok(Some(task)) => task,
        Ok(None) => {
            warn!("任务不存在: {}", task_id);
            return ApiResponse::not_found::<TaskResultSummaryResponse>(&format!(
                "任务不存在: {task_id}"
            ))
            .into_response();
        }
        Err(e) => {
            error!("获取任务详情失败: task_id={}, error={}", task_id, e);
            return ApiResponse::from_app_error::<TaskResultSummaryResponse>(e).into_response();
        }
    };

    // 构建文件信息
    let file_info = Some(TaskFileInfo {
        original_filename: task.original_filename.clone(),
        file_size: task.file_size,
        mime_type: task.mime_type.clone(),
        format: match &task.document_format {
            Some(fmt) => format!("{fmt:?}"),
            None => "Unknown".to_string(),
        },
    });

    // 构建OSS信息
    let oss_info = task.oss_data.as_ref().map(|oss| TaskOssInfo {
        bucket: oss.bucket.clone(),
        markdown_available: !oss.markdown_url.is_empty(),
        images_count: oss.images.len(),
    });

    // 构建处理统计信息
    let processing_stats = match &task.status {
        TaskStatus::Completed {
            processing_time, ..
        } => {
            Some(TaskProcessingStats {
                processing_time: Some(format!("{}ms", processing_time.as_millis())),
                word_count: None, // TODO: 从结果数据中提取
                page_count: None, // TODO: 从结果数据中提取
            })
        }
        _ => None,
    };

    let response = TaskResultSummaryResponse {
        task_id: task.id.clone(),
        status: task.status.clone(),
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
        file_info,
        oss_info,
        processing_stats,
    };

    info!("获取任务结果概览成功: {}", task_id);
    ApiResponse::success(response).into_response()
}
