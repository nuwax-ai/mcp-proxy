use axum::{
    extract::{Multipart, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::app_state::AppState;
use crate::error::AppError;
use crate::handlers::response::ApiResponse;

/// 文件上传响应
#[derive(Debug, Serialize)]
pub struct FileUploadResponse {
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub download_url: String,
    pub expires_in_hours: u64,
    pub file_size: Option<u64>,
    pub original_filename: Option<String>,
}

/// 下载URL响应
#[derive(Debug, Serialize)]
pub struct DownloadUrlResponse {
    pub download_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
}

/// 获取下载URL请求参数
#[derive(Debug, Deserialize)]
pub struct GetDownloadUrlParams {
    pub file_name: String,
    pub bucket: Option<String>,
}

/// 获取上传签名URL请求参数
#[derive(Debug, Deserialize)]
pub struct GetUploadSignUrlParams {
    pub file_name: String,
    pub content_type: Option<String>,
    pub bucket: Option<String>,
}

/// 获取下载签名URL请求参数（4小时有效）
#[derive(Debug, Deserialize)]
pub struct GetDownloadSignUrlParams {
    pub file_name: String,
    pub bucket: Option<String>,
}

/// 上传签名URL响应
#[derive(Debug, Serialize)]
pub struct UploadSignUrlResponse {
    pub upload_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
    pub content_type: String,
}

/// 下载签名URL响应
#[derive(Debug, Serialize)]
pub struct DownloadSignUrlResponse {
    pub download_url: String,
    pub oss_file_name: String,
    pub oss_bucket: String,
    pub expires_in_hours: u64,
}

/// 上传文件到OSS
pub async fn upload_file_to_oss(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    info!("OSS文件上传请求");

    // 检查OSS服务是否可用
    let oss_service = match &state.oss_service {
        Some(service) => service,
        None => {
            error!("OSS服务未配置");
            return ApiResponse::internal_error::<FileUploadResponse>("OSS服务未配置")
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

    // 上传到OSS
    match oss_service
        .upload_user_file(&file_path, original_filename.as_deref())
        .await
    {
        Ok((_oss_url, object_key, download_url)) => {
            info!("文件上传OSS成功: object_key={}", object_key);

            let response = FileUploadResponse {
                oss_file_name: object_key,
                oss_bucket: state.config.storage.oss.bucket.clone(),
                download_url,
                expires_in_hours: 4,
                file_size,
                original_filename,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!("文件上传OSS失败: {}", e);
            ApiResponse::from_app_error::<FileUploadResponse>(e).into_response()
        }
    }
}

/// 根据文件名获取下载URL
pub async fn get_download_url(
    State(state): State<AppState>,
    Query(params): Query<GetDownloadUrlParams>,
) -> impl IntoResponse {
    info!(
        "获取下载URL请求: file_name={}, bucket={:?}",
        params.file_name, params.bucket
    );

    // 验证文件名
    if params.file_name.trim().is_empty() {
        return ApiResponse::validation_error::<DownloadUrlResponse>("文件名不能为空")
            .into_response();
    }

    // 检查OSS服务是否可用
    let oss_service = match &state.oss_service {
        Some(service) => service,
        None => {
            error!("OSS服务未配置");
            return ApiResponse::internal_error::<DownloadUrlResponse>("OSS服务未配置")
                .into_response();
        }
    };

    // 根据是否指定bucket来调用不同的方法
    let result = if let Some(bucket) = &params.bucket {
        // 使用指定的bucket
        oss_service
            .get_download_url_for_file_with_bucket(&params.file_name, bucket)
            .await
    } else {
        // 使用默认bucket
        oss_service
            .get_download_url_for_file(&params.file_name)
            .await
    };

    // 处理结果
    match result {
        Ok(download_url) => {
            info!(
                "生成下载URL成功: file_name={}, bucket={:?}",
                params.file_name, params.bucket
            );

            // 确定使用的bucket名称
            let actual_bucket = params
                .bucket
                .as_ref()
                .unwrap_or(&state.config.storage.oss.bucket)
                .clone();

            let response = DownloadUrlResponse {
                download_url,
                oss_file_name: params.file_name.clone(),
                oss_bucket: actual_bucket,
                expires_in_hours: 4,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) if e.to_string().contains("文件不存在") => {
            warn!(
                "文件不存在: file_name={}, bucket={:?}",
                params.file_name, params.bucket
            );
            ApiResponse::not_found::<DownloadUrlResponse>(&format!(
                "文件不存在: {}",
                params.file_name
            ))
            .into_response()
        }
        Err(e) => {
            error!(
                "获取下载URL失败: file_name={}, bucket={:?}, error={}",
                params.file_name, params.bucket, e
            );
            ApiResponse::from_app_error::<DownloadUrlResponse>(e).into_response()
        }
    }
}

/// 获取上传签名URL
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

    // 检查OSS服务是否可用
    let oss_service = match &state.oss_service {
        Some(service) => service,
        None => {
            error!("OSS服务未配置");
            return ApiResponse::internal_error::<UploadSignUrlResponse>("OSS服务未配置")
                .into_response();
        }
    };

    // 默认内容类型
    let content_type = params
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    // 4小时有效期
    let expires_in = std::time::Duration::from_secs(4 * 3600);

    // 构建完整的对象键名，包含配置的子目录前缀
    let object_key = if params.file_name.starts_with(&state.config.storage.oss.upload_directory) {
        // 如果已经包含前缀，直接使用
        params.file_name.clone()
    } else {
        // 添加配置的子目录前缀
        format!("{}/{}", state.config.storage.oss.upload_directory, params.file_name.trim_start_matches('/'))
    };

    // 生成上传签名URL
    let result = if params.content_type.is_some() {
        oss_service
            .generate_upload_url_with_content_type(
                &object_key,
                content_type,
                Some(expires_in),
            )
            .await
    } else {
        oss_service
            .generate_upload_url(&object_key, Some(expires_in))
            .await
    };

    match result {
        Ok(upload_url) => {
            info!(
                "生成上传签名URL成功: object_key={}, content_type={}",
                object_key, content_type
            );

            // 确定使用的bucket名称
            let actual_bucket = params
                .bucket
                .as_ref()
                .unwrap_or(&state.config.storage.oss.bucket)
                .clone();

            let response = UploadSignUrlResponse {
                upload_url,
                oss_file_name: object_key,
                oss_bucket: actual_bucket,
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
            ApiResponse::from_app_error::<UploadSignUrlResponse>(e).into_response()
        }
    }
}

/// 获取下载签名URL（4小时有效）
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

    // 检查OSS服务是否可用
    let oss_service = match &state.oss_service {
        Some(service) => service,
        None => {
            error!("OSS服务未配置");
            return ApiResponse::internal_error::<DownloadSignUrlResponse>("OSS服务未配置")
                .into_response();
        }
    };

    // 4小时有效期
    let expires_in = std::time::Duration::from_secs(4 * 3600);

    // 构建完整的对象键名，如果没有前缀则添加配置的子目录前缀
    let object_key = if params.file_name.starts_with(&state.config.storage.oss.upload_directory) {
        // 如果已经包含前缀，直接使用
        params.file_name.clone()
    } else {
        // 添加配置的子目录前缀
        format!("{}/{}", state.config.storage.oss.upload_directory, params.file_name.trim_start_matches('/'))
    };

    // 生成下载签名URL
    let result = oss_service
        .generate_download_url(&object_key, Some(expires_in))
        .await;

    match result {
        Ok(download_url) => {
            info!("生成下载签名URL成功: object_key={}", object_key);

            // 确定使用的bucket名称
            let actual_bucket = params
                .bucket
                .as_ref()
                .unwrap_or(&state.config.storage.oss.bucket)
                .clone();

            let response = DownloadSignUrlResponse {
                download_url,
                oss_file_name: object_key,
                oss_bucket: actual_bucket,
                expires_in_hours: 4,
            };

            ApiResponse::success(response).into_response()
        }
        Err(e) => {
            error!(
                "生成下载签名URL失败: file_name={}, error={}",
                params.file_name, e
            );
            ApiResponse::from_app_error::<DownloadSignUrlResponse>(e).into_response()
        }
    }
}
