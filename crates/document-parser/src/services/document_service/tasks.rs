//! `DocumentService` 的任务生命周期族方法（从 document_service.rs 拆出）：
//! 任务状态安全更新（×6）与上传/URL 任务创建、状态查询。纯代码搬移，无行为变化。

use crate::config::GlobalFileSizeConfig;
use crate::error::AppError;
use crate::models::{DocumentFormat, ParserEngine, SourceType, TaskStatus};
use anyhow::{Context, Result as AnyhowResult};
use oss_client::ApiFileClient;
use std::path::Path;

use crate::services::upload_pipeline::{ResolvedUploader, TaskUploader};
use tracing::{info, instrument, warn};

impl super::DocumentService {
    /// Safe task update methods - handle errors gracefully without failing the main operation
    pub(super) async fn update_task_stage_safe(
        &self,
        task_id: &str,
        stage: crate::models::ProcessingStage,
    ) {
        if let Err(e) = self.task_service.update_task_stage(task_id, stage).await {
            warn!("Failed to update task stage for {}: {}", task_id, e);
        }
    }

    pub(super) async fn update_task_progress_safe(&self, task_id: &str, progress: u32) {
        if let Err(e) = self
            .task_service
            .update_task_progress(task_id, progress)
            .await
        {
            warn!("Failed to update task progress for {}: {}", task_id, e);
        }
    }

    pub(super) async fn update_task_file_info_safe(
        &self,
        task_id: &str,
        file_size: Option<u64>,
        mime_type: Option<String>,
    ) {
        if let Err(e) = self
            .task_service
            .set_task_file_info(task_id, file_size, mime_type)
            .await
        {
            warn!("Failed to update task file info for {}: {}", task_id, e);
        }
    }

    pub(super) async fn update_task_parser_engine_safe(&self, task_id: &str, engine: ParserEngine) {
        if let Err(e) = self
            .task_service
            .set_task_parser_engine(task_id, engine)
            .await
        {
            warn!("Failed to update task parser engine for {}: {}", task_id, e);
        }
    }

    pub(super) async fn update_task_document_format_safe(
        &self,
        task_id: &str,
        format: DocumentFormat,
    ) {
        if let Err(e) = self
            .task_service
            .update_task(task_id, None, None, format)
            .await
        {
            warn!(
                "Failed to update task document format for {}: {}",
                task_id, e
            );
        }
    }

    pub(super) async fn update_task_status_safe(&self, task_id: &str, status: TaskStatus) {
        if let Err(e) = self.task_service.update_task_status(task_id, status).await {
            warn!("Failed to update task status for {}: {}", task_id, e);
        }
    }

    /// 创建文件上传任务 - Enhanced with proper validation and error handling
    #[instrument(skip(self), fields(filename = %filename, file_size = file_size))]
    pub async fn create_upload_task(
        &self,
        file_path: &str,
        filename: &str,
        file_size: u64,
    ) -> AnyhowResult<String> {
        info!(
            "Create file upload task: {} (size: {} bytes)",
            filename, file_size
        );

        // Validate file size
        let global_config = GlobalFileSizeConfig::new();
        if file_size > global_config.max_file_size.bytes() {
            return Err(anyhow::anyhow!(
                "文件大小超过限制: {} > {} bytes",
                file_size,
                global_config.max_file_size.bytes()
            ));
        }

        // Validate file exists
        let file_path_obj = Path::new(file_path);
        if !file_path_obj.exists() {
            return Err(anyhow::anyhow!("文件不存在: {}", file_path));
        }

        // Create task
        let task = self
            .task_service
            .create_task(
                SourceType::Upload,
                Some(filename.to_string()),
                Some(filename.to_string()),
                None,
            )
            .await
            .context("创建任务失败")?;

        Ok(task.id)
    }

    /// 创建URL下载任务
    pub async fn create_url_task(&self, url: &str, filename: &str) -> Result<String, AppError> {
        log::info!("Create URL download task: {url} -> {filename}");

        // 创建任务：URL 作为 source_url，原始文件名保留
        let task = self
            .task_service
            .create_task(
                SourceType::Url,
                Some(url.to_string()),
                Some(filename.to_string()),
                None,
            )
            .await
            .map_err(|e| AppError::Task(format!("创建任务失败: {e}")))?;

        Ok(task.id)
    }

    /// 获取任务状态
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<crate::models::DocumentTask, AppError> {
        log::debug!("Get task status: {task_id}");

        self.task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))
    }

    /// 将处理后的Markdown内容上传到 OSS 或自定义上传后端
    ///
    /// 返回 `(url, key)`：OSS 时 key 为对象键；自定义后端时为服务端返回的文件 key。
    pub(crate) async fn resolve_task_uploader(
        &self,
        task_id: &str,
    ) -> AnyhowResult<ResolvedUploader> {
        let task = self
            .task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("任务不存在: {task_id}"))?;

        match task.upload_config {
            Some(endpoint) => {
                let client =
                    ApiFileClient::with_client(endpoint.to_api_config(), self.http_client.clone())
                        .map_err(|e| anyhow::anyhow!("自定义上传客户端初始化失败: {e}"))?;
                Ok(ResolvedUploader {
                    custom_base_url: Some(endpoint.base_url.clone()),
                    uploader: TaskUploader::Custom(client),
                    bucket_dir: None, // 自定义后端服务端自管 key，忽略 bucket_dir
                })
            }
            None => Ok(ResolvedUploader {
                custom_base_url: None,
                uploader: match &self.oss_client {
                    Some(client) => TaskUploader::Oss(client.clone()),
                    None => TaskUploader::Disabled,
                },
                bucket_dir: task
                    .bucket_dir
                    .as_ref()
                    .map(|dir| dir.trim_matches('/').to_string()),
            }),
        }
    }
}
