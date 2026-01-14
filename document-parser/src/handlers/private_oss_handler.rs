use axum::{
    extract::{Multipart, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, info, warn};
use utoipa::ToSchema;

use crate::app_state::AppState;
use crate::handlers::response::ApiResponse;

/// 文件上传响应
#[derive(Debug, Serialize, ToSchema)]
pub struct FileUploadResponse {
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub download_url: String,
    pub expires_in_hours: u64,
    pub file_size: Option<u64>,
    pub original_filename: Option<String>,
}

/// 下载URL响应
#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadUrlResponse {
    pub download_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
}

/// 获取下载URL请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetDownloadUrlParams {
    pub file_name: String,
    pub bucket: Option<String>,
}

/// 获取上传签名URL请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetUploadSignUrlParams {
    pub file_name: String,
    pub content_type: Option<String>,
    pub bucket: Option<String>,
}

/// 获取下载签名URL请求参数（4小时有效）
#[derive(Debug, Deserialize, ToSchema)]
pub struct GetDownloadSignUrlParams {
    pub file_name: String,
    pub bucket: Option<String>,
}

/// 删除文件请求参数
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteFileParams {
    pub file_name: String,
    pub bucket: Option<String>,
}

/// 上传签名URL响应
#[derive(Debug, Serialize, ToSchema)]
pub struct UploadSignUrlResponse {
    pub upload_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
    pub content_type: String,
}

/// 下载签名URL响应
#[derive(Debug, Serialize, ToSchema)]
pub struct DownloadSignUrlResponse {
    pub download_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
}

/// 删除文件响应
#[derive(Debug, Serialize, ToSchema)]
pub struct DeleteFileResponse {
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub message: String,
}

/// 上传文件到OSS
#[utoipa::path(
    post,
    path = "/api/v1/oss/upload",
    request_body(content = String, description = "文件内容", content_type = "multipart/form-data"),
    responses(
        (status = 200, description = "上传成功", body = FileUploadResponse),
        (status = 400, description = "请求参数错误"),
        (status = 413, description = "文件过大"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "oss"
)]
pub async fn upload_file_to_oss(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    info!("OSS文件上传请求");

    // 检查OSS客户端是否可用
    let oss_client = match &state.private_oss_client {
        Some(client) => client,
        None => {
            error!("OSS客户端未配置");
            return ApiResponse::internal_error::<FileUploadResponse>("OSS客户端未配置")
                .into_response();
        }
    };

    let mut file_path: Option<String> = None;
    let mut original_filename: Option<String> = None;
    let mut temp_files = Vec::new();

    // 处理multipart数据
    while let Some(field) = multipart.next_field().await.unwrap_or(None) {
        let field_name = field.name().unwrap_or("").to_string();

        if field_name == "file" {
            let filename = field.file_name().map(|s| s.to_string());
            original_filename = filename.clone();

            let data = match field.bytes().await {
                Ok(data) => data,
                Err(e) => {
                    error!("读取文件数据失败: {}", e);
                    return ApiResponse::validation_error::<FileUploadResponse>("文件数据读取失败")
                        .into_response();
                }
            };

            // 创建临时文件
            let temp_file = match tempfile::NamedTempFile::new() {
                Ok(file) => file,
                Err(e) => {
                    error!("创建临时文件失败: {}", e);
                    return ApiResponse::internal_error::<FileUploadResponse>("临时文件创建失败")
                        .into_response();
                }
            };

            if let Err(e) = std::fs::write(temp_file.path(), &data) {
                error!("写入临时文件失败: {}", e);
                return ApiResponse::internal_error::<FileUploadResponse>("文件写入失败")
                    .into_response();
            }

            file_path = Some(temp_file.path().to_string_lossy().to_string());
            temp_files.push(temp_file);
        }
    }

    let file_path = match file_path {
        Some(path) => path,
        None => {
            warn!("未找到上传的文件");
            return ApiResponse::validation_error::<FileUploadResponse>("未提供文件")
                .into_response();
        }
    };

    // 获取文件大小
    let file_size = std::fs::metadata(&file_path).map(|m| m.len()).ok();

    // 生成对象键名
    let object_key = if let Some(ref filename) = original_filename {
        // 使用原始文件名，添加时间戳避免冲突
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let clean_filename = oss_client::utils::sanitize_filename(filename);
        format!("uploads/{timestamp}_{clean_filename}")
    } else {
        // 生成唯一文件名
        format!(
            "uploads/{}",
            oss_client::utils::generate_random_filename(None)
        )
    };

    // 上传到OSS
    match oss_client.upload_file(&file_path, &object_key).await {
        Ok(oss_url) => {
            info!("文件上传OSS成功: object_key={}", object_key);

            // 生成下载URL（4小时有效）
            let download_url = match oss_client
                .generate_download_url(&object_key, Some(Duration::from_secs(4 * 3600)))
            {
                Ok(url) => url,
                Err(_) => oss_url.clone(), // 如果生成签名URL失败，使用原始URL
            };

            let response = FileUploadResponse {
                oss_file_name: object_key,
                oss_bucket: oss_client.get_config().bucket.clone(),
                download_url,
                expires_in_hours: 4,
                file_size,
                original_filename,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("文件上传OSS失败: {}", e);
            ApiResponse::internal_error::<FileUploadResponse>(&format!("上传失败: {e}"))
                .into_response()
        }
    }
}

/// 获取上传签名URL
///
/// ⚠️ **重要警告**: 使用相同的文件名上传会完全覆盖OSS中的现有文件，此操作不可逆！
/// 建议在文件名中添加时间戳或UUID来避免意外覆盖，例如：document_20240101_120000.pdf
#[utoipa::path(
    get,
    path = "/api/v1/oss/upload-sign-url",
    params(
        ("file_name" = String, Query, description = "文件名（⚠️警告：相同文件名会覆盖现有文件！建议添加时间戳避免覆盖）"),
        ("content_type" = Option<String>, Query, description = "文件内容类型，默认为 application/octet-stream"),
        ("bucket" = Option<String>, Query, description = "存储桶名称（可选）")
    ),
    responses(
        (status = 200, description = "获取成功，返回4小时有效的上传签名URL", body = UploadSignUrlResponse),
        (status = 400, description = "请求参数错误（如文件名为空）"),
        (status = 500, description = "服务器内部错误（如OSS客户端未配置）")
    ),
    tag = "oss"
)]
pub async fn get_upload_sign_url(
    State(state): State<AppState>,
    Query(params): Query<GetUploadSignUrlParams>,
) -> impl IntoResponse {
    info!(
        "获取上传签名URL请求: file_name={}, content_type={:?}, bucket={:?}",
        params.file_name, params.content_type, params.bucket
    );

    // 验证文件名
    if params.file_name.trim().is_empty() {
        return ApiResponse::validation_error::<UploadSignUrlResponse>("文件名不能为空")
            .into_response();
    }

    // 检查OSS客户端是否可用
    let oss_client = match &state.private_oss_client {
        Some(client) => client,
        None => {
            error!("OSS客户端未配置");
            return ApiResponse::internal_error::<UploadSignUrlResponse>("OSS客户端未配置")
                .into_response();
        }
    };

    // 默认内容类型
    let content_type = params
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    // 4小时有效期
    let expires_in = Duration::from_secs(4 * 3600);

    // 构建完整的对象键名，包含edu前缀
    let object_key = if params.file_name.starts_with("edu/") {
        // 如果已经包含前缀，直接使用
        params.file_name.clone()
    } else {
        // 添加edu前缀
        format!("edu/{}", params.file_name.trim_start_matches('/'))
    };

    // 生成上传签名URL
    match oss_client.generate_upload_url(&object_key, expires_in, Some(content_type)) {
        Ok(upload_url) => {
            info!(
                "生成上传签名URL成功: object_key={}, content_type={}",
                object_key, content_type
            );

            let response = UploadSignUrlResponse {
                upload_url,
                oss_file_name: object_key,
                oss_bucket: oss_client.get_config().bucket.clone(),
                expires_in_hours: 4,
                content_type: content_type.to_string(),
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!(
                "生成上传签名URL失败: file_name={}, error={}",
                params.file_name, e
            );
            ApiResponse::internal_error::<UploadSignUrlResponse>(&format!(
                "生成上传签名URL失败: {e}"
            ))
            .into_response()
        }
    }
}

/// 获取下载签名URL（4小时有效）
#[utoipa::path(
    get,
    path = "/api/v1/oss/download-sign-url",
    params(
        ("file_name" = String, Query, description = "文件名"),
        ("bucket" = Option<String>, Query, description = "存储桶名称")
    ),
    responses(
        (status = 200, description = "获取成功", body = DownloadSignUrlResponse),
        (status = 400, description = "请求参数错误"),
        (status = 404, description = "文件不存在"),
        (status = 500, description = "服务器内部错误")
    ),
    tag = "oss"
)]
pub async fn get_download_sign_url(
    State(state): State<AppState>,
    Query(params): Query<GetDownloadSignUrlParams>,
) -> impl IntoResponse {
    info!(
        "获取下载签名URL请求: file_name={}, bucket={:?}",
        params.file_name, params.bucket
    );

    // 验证文件名
    if params.file_name.trim().is_empty() {
        return ApiResponse::validation_error::<DownloadSignUrlResponse>("文件名不能为空")
            .into_response();
    }

    // 检查OSS客户端是否可用
    let oss_client = match &state.private_oss_client {
        Some(client) => client,
        None => {
            error!("OSS客户端未配置");
            return ApiResponse::internal_error::<DownloadSignUrlResponse>("OSS客户端未配置")
                .into_response();
        }
    };

    // 4小时有效期
    let expires_in = Duration::from_secs(4 * 3600);

    // 构建完整的对象键名，如果没有前缀则添加edu前缀
    let object_key = if params.file_name.starts_with("edu/") {
        // 如果已经包含前缀，直接使用
        params.file_name.clone()
    } else {
        // 添加edu前缀
        format!("edu/{}", params.file_name.trim_start_matches('/'))
    };

    // 生成下载签名URL
    match oss_client.generate_download_url(&object_key, Some(expires_in)) {
        Ok(download_url) => {
            info!("生成下载签名URL成功: object_key={}", object_key);

            let response = DownloadSignUrlResponse {
                download_url,
                oss_file_name: object_key,
                oss_bucket: oss_client.get_config().bucket.clone(),
                expires_in_hours: 4,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!(
                "生成下载签名URL失败: file_name={}, error={}",
                params.file_name, e
            );
            ApiResponse::internal_error::<DownloadSignUrlResponse>(&format!(
                "生成下载签名URL失败: {e}"
            ))
            .into_response()
        }
    }
}

/// 删除OSS文件
#[utoipa::path(
    get,
    path = "/api/v1/oss/delete",
    params(
        ("file_name" = String, Query, description = "要删除的文件名"),
        ("bucket" = Option<String>, Query, description = "存储桶名称（可选）")
    ),
    responses(
        (status = 200, description = "删除成功", body = DeleteFileResponse),
        (status = 400, description = "请求参数错误（如文件名为空）"),
        (status = 404, description = "文件不存在"),
        (status = 500, description = "服务器内部错误（如OSS客户端未配置）")
    ),
    tag = "oss"
)]
pub async fn delete_file_from_oss(
    State(state): State<AppState>,
    Query(params): Query<DeleteFileParams>,
) -> impl IntoResponse {
    info!(
        "删除OSS文件请求: file_name={}, bucket={:?}",
        params.file_name, params.bucket
    );

    // 验证文件名
    if params.file_name.trim().is_empty() {
        return ApiResponse::validation_error::<DeleteFileResponse>("文件名不能为空")
            .into_response();
    }

    // 检查OSS客户端是否可用
    let oss_client = match &state.private_oss_client {
        Some(client) => client,
        None => {
            error!("OSS客户端未配置");
            return ApiResponse::internal_error::<DeleteFileResponse>("OSS客户端未配置")
                .into_response();
        }
    };

    // 构建完整的对象键名，如果没有前缀则添加edu前缀
    let object_key = if params.file_name.starts_with("edu/") {
        // 如果已经包含前缀，直接使用
        params.file_name.clone()
    } else {
        // 添加edu前缀
        format!("edu/{}", params.file_name.trim_start_matches('/'))
    };

    // 先检查文件是否存在
    match oss_client.file_exists(&object_key).await {
        Ok(exists) => {
            if !exists {
                warn!("要删除的文件不存在: {}", object_key);
                return ApiResponse::not_found::<DeleteFileResponse>("文件不存在").into_response();
            }
        }
        Err(e) => {
            error!(
                "检查文件存在性失败: file_name={}, error={}",
                params.file_name, e
            );
            return ApiResponse::internal_error::<DeleteFileResponse>(&format!(
                "检查文件存在性失败: {e}"
            ))
            .into_response();
        }
    }

    // 删除文件
    match oss_client.delete_file(&object_key).await {
        Ok(_) => {
            info!("删除OSS文件成功: object_key={}", object_key);

            let response = DeleteFileResponse {
                oss_file_name: object_key,
                oss_bucket: oss_client.get_config().bucket.clone(),
                message: "文件删除成功".to_string(),
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!(
                "删除OSS文件失败: file_name={}, error={}",
                params.file_name, e
            );
            ApiResponse::internal_error::<DeleteFileResponse>(&format!("删除文件失败: {e}"))
                .into_response()
        }
    }
}
