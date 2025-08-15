use crate::error::AppError;
use crate::models::{DocumentFormat, OssData, ParserEngine, StructuredDocument, TaskStatus, TaskError, ProcessingStage};
use crate::config::{get_global_file_size_config, GlobalFileSizeConfig, FileSizePurpose};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 任务数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentTask {
    pub id: String,
    pub status: TaskStatus,
    pub source_type: SourceType,
    pub source_path: Option<String>,
    pub document_format: DocumentFormat,
    pub parser_engine: ParserEngine,
    pub backend: String,
    pub progress: u32,
    pub error_message: Option<String>,
    pub oss_data: Option<OssData>,
    pub structured_document: Option<StructuredDocument>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
    pub retry_count: u32,
    pub max_retries: u32,
}

/// 任务来源类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceType {
    Upload,      // 文件上传
    Url,         // URL下载
    ExternalApi, // 外部API调用
    Oss,         // OSS文件
}

/// DocumentTask构建器
#[derive(Debug, Default)]
pub struct DocumentTaskBuilder {
    id: Option<String>,
    source_type: Option<SourceType>,
    source_path: Option<String>,
    document_format: Option<DocumentFormat>,
    parser_engine: Option<ParserEngine>,
    backend: Option<String>,
    file_size: Option<u64>,
    mime_type: Option<String>,
    expires_in_hours: Option<i64>,
    max_retries: Option<u32>,
}

impl DocumentTask {
    /// 创建构建器
    pub fn builder() -> DocumentTaskBuilder {
        DocumentTaskBuilder::default()
    }

    /// 创建新的任务（保持向后兼容）
    pub fn new(
        id: String,
        source_type: SourceType,
        source_path: Option<String>,
        document_format: DocumentFormat,
    ) -> Self {
        Self::builder()
            .id(id)
            .source_type(source_type)
            .source_path(source_path)
            .document_format(document_format)
            .build()
            .expect("Failed to build DocumentTask with valid parameters")
    }

    /// 验证任务数据完整性
    pub fn validate(&self) -> Result<(), AppError> {
        // 验证ID格式
        if self.id.is_empty() {
            return Err(AppError::Validation("任务ID不能为空".to_string()));
        }

        // 验证UUID格式
        if Uuid::parse_str(&self.id).is_err() {
            return Err(AppError::Validation(
                "任务ID必须是有效的UUID格式".to_string(),
            ));
        }

        // 验证文档格式支持
        if !self.document_format.is_supported() {
            return Err(AppError::UnsupportedFormat(format!(
                "不支持的文档格式: {}",
                self.document_format
            )));
        }

        // 验证解析引擎与格式匹配
        if !self.parser_engine.supports_format(&self.document_format) {
            return Err(AppError::Validation(format!(
                "解析引擎 {} 不支持格式 {}",
                self.parser_engine.get_name(),
                self.document_format
            )));
        }

        // 验证进度范围
        if self.progress > 100 {
            return Err(AppError::Validation("进度值不能超过100".to_string()));
        }

        // 验证文件大小
        if let Some(file_size) = self.file_size {
            if file_size == 0 {
                return Err(AppError::Validation("文件大小不能为0".to_string()));
            }
            let config = get_global_file_size_config();
            let max_size = config.get_max_size_for(FileSizePurpose::DocumentParser);
            if file_size > max_size {
                return Err(AppError::Validation(format!(
                    "文件大小 {} 字节超过最大限制 {} 字节",
                    file_size, max_size
                )));
            }
        }

        // 验证重试次数
        if self.retry_count > self.max_retries {
            return Err(AppError::Validation(format!(
                "重试次数 {} 超过最大限制 {}",
                self.retry_count, self.max_retries
            )));
        }

        // 验证时间逻辑
        if self.created_at > self.updated_at {
            return Err(AppError::Validation("创建时间不能晚于更新时间".to_string()));
        }

        if self.expires_at <= self.created_at {
            return Err(AppError::Validation("过期时间必须晚于创建时间".to_string()));
        }

        Ok(())
    }

    /// 验证状态转换是否合法
    pub fn validate_status_transition(&self, new_status: &TaskStatus) -> Result<(), AppError> {
        let valid_transition = match (&self.status, new_status) {
            // 从Pending可以转换到任何状态
            (TaskStatus::Pending { .. }, _) => true,

            // 从Processing只能转换到Completed、Failed或Cancelled
            (TaskStatus::Processing { .. }, TaskStatus::Completed { .. }) => true,
            (TaskStatus::Processing { .. }, TaskStatus::Failed { .. }) => true,
            (TaskStatus::Processing { .. }, TaskStatus::Cancelled { .. }) => true,
            (TaskStatus::Processing { .. }, TaskStatus::Processing { .. }) => true, // 允许阶段更新

            // 终态不能转换到其他状态（除非重置）
            (TaskStatus::Completed { .. }, _) => false,
            (TaskStatus::Cancelled { .. }, _) => false,

            // Failed可以重置到Pending（重试）
            (
                TaskStatus::Failed {
                    is_recoverable: true,
                    ..
                },
                TaskStatus::Pending { .. },
            ) => self.can_retry(),
            (TaskStatus::Failed { .. }, _) => false,

            // 其他转换无效
            _ => false,
        };

        if !valid_transition {
            return Err(AppError::Task(format!(
                "无效的状态转换: {} -> {}",
                self.status, new_status
            )));
        }

        Ok(())
    }

    /// 更新任务状态（带验证）
    pub fn update_status(&mut self, status: TaskStatus) -> Result<(), AppError> {
        self.validate_status_transition(&status)?;

        self.status = status;
        self.updated_at = Utc::now();

        // 如果是失败状态，增加重试计数
        if matches!(self.status, TaskStatus::Failed { .. }) {
            self.retry_count += 1;
        }

        Ok(())
    }

    /// 更新进度（带验证）
    pub fn update_progress(&mut self, progress: u32) -> Result<(), AppError> {
        if progress > 100 {
            return Err(AppError::Validation("进度值不能超过100".to_string()));
        }

        self.progress = progress;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 设置错误信息（带验证）
    pub fn set_error(&mut self, error: String) -> Result<(), AppError> {
        if error.is_empty() {
            return Err(AppError::Validation("错误信息不能为空".to_string()));
        }

        self.error_message = Some(error.clone());

        // 创建TaskError
        let task_error = TaskError::new(
            "E010".to_string(), // Task error code
            error,
            self.status.get_current_stage().cloned(),
        );

        let failed_status = TaskStatus::new_failed(task_error, self.retry_count);
        self.update_status(failed_status)?;
        Ok(())
    }

    /// 设置OSS数据（带验证）
    pub fn set_oss_data(&mut self, oss_data: OssData) -> Result<(), AppError> {
        // 这里可以添加OSS数据的验证逻辑
        self.oss_data = Some(oss_data);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 设置结构化文档（带验证）
    pub fn set_structured_document(&mut self, doc: StructuredDocument) -> Result<(), AppError> {
        // 验证文档ID匹配
        if doc.task_id != self.id {
            return Err(AppError::Validation(
                "结构化文档的任务ID与当前任务不匹配".to_string(),
            ));
        }

        self.structured_document = Some(doc);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 设置文件信息（带验证）
    pub fn set_file_info(&mut self, file_size: u64, mime_type: String) -> Result<(), AppError> {
        let config = get_global_file_size_config();
        let max_size = config.get_max_size_for(FileSizePurpose::DocumentParser);
        if file_size > max_size {
            return Err(AppError::Validation(format!(
                "文件大小 {} 字节超过最大限制 {} 字节",
                file_size, max_size
            )));
        }

        if mime_type.is_empty() {
            return Err(AppError::Validation("MIME类型不能为空".to_string()));
        }

        self.file_size = Some(file_size);
        self.mime_type = Some(mime_type);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 检查任务是否过期
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// 获取任务年龄（小时）
    pub fn get_age_hours(&self) -> i64 {
        let duration = Utc::now() - self.created_at;
        duration.num_hours()
    }

    /// 获取任务状态描述
    pub fn get_status_description(&self) -> String {
        self.status.get_description()
    }

    /// 检查任务是否可以重试
    pub fn can_retry(&self) -> bool {
        self.status.can_retry() && !self.is_expired() && self.retry_count < self.max_retries
    }

    /// 重置任务状态（用于重试）
    pub fn reset(&mut self) -> Result<(), AppError> {
        if !self.can_retry() {
            return Err(AppError::Task("任务不能重试".to_string()));
        }

        self.status = TaskStatus::new_pending();
        self.progress = 0;
        self.error_message = None;
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 取消任务
    pub fn cancel(&mut self) -> Result<(), AppError> {
        if self.status.is_terminal() && !self.status.is_failed() {
            return Err(AppError::Task("已完成的任务不能取消".to_string()));
        }

        self.update_status(TaskStatus::new_cancelled(Some("用户取消".to_string())))?;
        Ok(())
    }

    /// 获取剩余过期时间（小时）
    pub fn get_remaining_hours(&self) -> i64 {
        let remaining = self.expires_at - Utc::now();
        remaining.num_hours().max(0)
    }

    /// 延长过期时间
    pub fn extend_expiry(&mut self, hours: i64) -> Result<(), AppError> {
        if hours <= 0 {
            return Err(AppError::Validation("延长时间必须大于0".to_string()));
        }

        const MAX_EXTENSION_HOURS: i64 = 168; // 7天
        if hours > MAX_EXTENSION_HOURS {
            return Err(AppError::Validation(format!(
                "延长时间不能超过{}小时",
                MAX_EXTENSION_HOURS
            )));
        }

        self.expires_at = self.expires_at + Duration::hours(hours);
        self.updated_at = Utc::now();
        Ok(())
    }
}

impl DocumentTaskBuilder {
    /// 设置任务ID
    pub fn id<S: Into<String>>(mut self, id: S) -> Self {
        self.id = Some(id.into());
        self
    }

    /// 生成新的UUID作为任务ID
    pub fn generate_id(mut self) -> Self {
        self.id = Some(Uuid::new_v4().to_string());
        self
    }

    /// 设置来源类型
    pub fn source_type(mut self, source_type: SourceType) -> Self {
        self.source_type = Some(source_type);
        self
    }

    /// 设置来源路径
    pub fn source_path<S: Into<String>>(mut self, path: Option<S>) -> Self {
        self.source_path = path.map(|p| p.into());
        self
    }

    /// 设置文档格式
    pub fn document_format(mut self, format: DocumentFormat) -> Self {
        self.document_format = Some(format);
        self
    }

    /// 设置解析引擎
    pub fn parser_engine(mut self, engine: ParserEngine) -> Self {
        self.parser_engine = Some(engine);
        self
    }

    /// 设置后端
    pub fn backend<S: Into<String>>(mut self, backend: S) -> Self {
        self.backend = Some(backend.into());
        self
    }

    /// 设置文件大小
    pub fn file_size(mut self, size: u64) -> Self {
        self.file_size = Some(size);
        self
    }

    /// 设置MIME类型
    pub fn mime_type<S: Into<String>>(mut self, mime_type: S) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }

    /// 设置过期时间（小时）
    pub fn expires_in_hours(mut self, hours: i64) -> Self {
        self.expires_in_hours = Some(hours);
        self
    }

    /// 设置最大重试次数
    pub fn max_retries(mut self, retries: u32) -> Self {
        self.max_retries = Some(retries);
        self
    }

    /// 构建DocumentTask
    pub fn build(self) -> Result<DocumentTask, AppError> {
        let id = self.id.unwrap_or_else(|| Uuid::new_v4().to_string());

        let source_type = self
            .source_type
            .ok_or_else(|| AppError::Validation("必须指定来源类型".to_string()))?;

        let document_format = self
            .document_format
            .ok_or_else(|| AppError::Validation("必须指定文档格式".to_string()))?;

        let parser_engine = self
            .parser_engine
            .unwrap_or_else(|| ParserEngine::select_for_format(&document_format));

        let backend = self.backend.unwrap_or_else(|| "default".to_string());
        let expires_in_hours = self.expires_in_hours.unwrap_or(24);
        let max_retries = self.max_retries.unwrap_or(3);

        let now = Utc::now();

        let task = DocumentTask {
            id,
            status: TaskStatus::new_pending(),
            source_type,
            source_path: self.source_path,
            document_format,
            parser_engine,
            backend,
            progress: 0,
            error_message: None,
            oss_data: None,
            structured_document: None,
            created_at: now,
            updated_at: now,
            expires_at: now + Duration::hours(expires_in_hours),
            file_size: self.file_size,
            mime_type: self.mime_type,
            retry_count: 0,
            max_retries,
        };

        // 验证构建的任务
        task.validate()?;

        Ok(task)
    }
}

impl SourceType {
    /// 获取来源类型描述
    pub fn get_description(&self) -> &'static str {
        match self {
            SourceType::Upload => "文件上传",
            SourceType::Url => "URL下载",
            SourceType::ExternalApi => "外部API调用",
            SourceType::Oss => "OSS文件",
        }
    }

    /// 验证来源路径是否有效
    pub fn validate_source_path(&self, path: &Option<String>) -> Result<(), AppError> {
        match self {
            SourceType::Upload => {
                if let Some(p) = path {
                    if p.is_empty() {
                        return Err(AppError::Validation("文件上传路径不能为空".to_string()));
                    }
                    // 可以添加更多文件路径验证逻辑
                }
            }
            SourceType::Url => {
                if let Some(url) = path {
                    if url.is_empty() {
                        return Err(AppError::Validation("下载URL不能为空".to_string()));
                    }
                    // 简单的URL格式验证
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        return Err(AppError::Validation(
                            "URL必须以http://或https://开头".to_string(),
                        ));
                    }
                } else {
                    return Err(AppError::Validation("URL下载必须提供URL".to_string()));
                }
            }
            SourceType::ExternalApi => {
                // 外部API调用的验证逻辑
                if let Some(api_path) = path {
                    if api_path.is_empty() {
                        return Err(AppError::Validation("API路径不能为空".to_string()));
                    }
                }
            }
            SourceType::Oss => {
                // OSS文件的验证逻辑
                if let Some(oss_path) = path {
                    if oss_path.is_empty() {
                        return Err(AppError::Validation("OSS文件路径不能为空".to_string()));
                    }
                } else {
                    return Err(AppError::Validation("OSS文件必须提供文件路径".to_string()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{DocumentFormat, ParserEngine, ProcessingStage, TaskStatus};
    use crate::config::{init_global_config, AppConfig};
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn init_test_config() {
        INIT.call_once(|| {
            let config = AppConfig::load_base_config().unwrap();
            init_global_config(config).unwrap();
        });
    }

    #[test]
    fn test_document_task_builder_success() {
        init_test_config();
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .source_path(Some("/path/to/file.pdf"))
            .document_format(DocumentFormat::PDF)
            .file_size(1024)
            .mime_type("application/pdf")
            .build()
            .expect("Should build successfully");

        assert!(!task.id.is_empty());
        assert_eq!(task.source_type, SourceType::Upload);
        assert_eq!(task.document_format, DocumentFormat::PDF);
        assert_eq!(task.parser_engine, ParserEngine::MinerU);
        assert_eq!(task.file_size, Some(1024));
        assert_eq!(task.mime_type, Some("application/pdf".to_string()));
        assert_eq!(task.retry_count, 0);
        assert_eq!(task.max_retries, 3);
    }

    #[test]
    fn test_document_task_builder_missing_required_fields() {
        init_test_config();
        let result = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            // Missing document_format
            .build();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AppError::Validation(_)));
    }

    #[test]
    fn test_document_task_validation_success() {
        init_test_config();
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        assert!(task.validate().is_ok());
    }

    #[test]
    fn test_document_task_validation_invalid_uuid() {
        init_test_config();
        let result = DocumentTask::builder()
            .id("invalid-uuid")
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build();

        // Should fail during build due to invalid UUID
        assert!(result.is_err());
    }

    #[test]
    fn test_document_task_validation_unsupported_format() {
        init_test_config();
        let result = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::Other("unknown".to_string()))
            .build();

        // Should fail during build due to unsupported format
        assert!(result.is_err());
    }

    #[test]
    fn test_document_task_validation_engine_format_mismatch() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::Word)
            .build()
            .unwrap();

        // Manually set mismatched engine
        task.parser_engine = ParserEngine::MinerU;
        assert!(task.validate().is_err());
    }

    #[test]
    fn test_document_task_validation_file_size_too_large() {
        init_test_config();
        
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .file_size(120 * 1024 * 1024) // 120MB > 100MB limit (from config.yml)
            .build();

        assert!(task.is_err());
    }

    #[test]
    fn test_document_task_validation_file_size_within_limit() {
        init_test_config();
        
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .file_size(50 * 1024 * 1024) // 50MB < 100MB limit (from config.yml)
            .build();

        assert!(task.is_ok());
    }

    #[test]
    fn test_status_transition_validation() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Valid transitions
        assert!(
            task.update_status(TaskStatus::Processing {
                stage: ProcessingStage::FormatDetection,
                started_at: Utc::now(),
                progress_details: None,
            })
            .is_ok()
        );

        assert!(task.update_status(TaskStatus::new_completed(std::time::Duration::from_secs(60))).is_ok());

        // Invalid transition from completed
        assert!(task.update_status(TaskStatus::new_pending()).is_err());
    }

    #[test]
    fn test_status_transition_failed_to_pending() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .max_retries(3)
            .build()
            .unwrap();

        // Set to failed status
        task.set_error("Test error".to_string()).unwrap();
        assert!(matches!(task.status, TaskStatus::Failed { .. }));
        assert_eq!(task.retry_count, 1);

        // Should be able to retry
        assert!(task.can_retry());
        assert!(task.update_status(TaskStatus::new_pending()).is_ok());
    }

    #[test]
    fn test_retry_limit_exceeded() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .max_retries(2)
            .build()
            .unwrap();

        // Exceed retry limit
        task.retry_count = 3;
        task.status = TaskStatus::Failed {
            error: TaskError::new(
                "E001".to_string(),
                "Test error".to_string(),
                None,
            ),
            failed_at: Utc::now(),
            retry_count: 0,
            is_recoverable: false,
        };

        assert!(!task.can_retry());
        assert!(task.update_status(TaskStatus::new_pending()).is_err());
    }

    #[test]
    fn test_update_progress_validation() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Valid progress
        assert!(task.update_progress(50).is_ok());
        assert_eq!(task.progress, 50);

        // Invalid progress
        assert!(task.update_progress(150).is_err());
    }

    #[test]
    fn test_set_error_validation() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Valid error
        assert!(task.set_error("Test error".to_string()).is_ok());
        assert!(matches!(task.status, TaskStatus::Failed { .. }));
        assert_eq!(task.error_message, Some("Test error".to_string()));

        // Invalid empty error
        let mut task2 = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();
        assert!(task2.set_error("".to_string()).is_err());
    }

    #[test]
    fn test_set_file_info_validation() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Valid file info
        assert!(
            task.set_file_info(1024, "application/pdf".to_string())
                .is_ok()
        );
        assert_eq!(task.file_size, Some(1024));
        assert_eq!(task.mime_type, Some("application/pdf".to_string()));

        // Invalid file size
        assert!(
            task.set_file_info(600 * 1024 * 1024, "application/pdf".to_string())
                .is_err()
        );

        // Invalid empty mime type
        assert!(task.set_file_info(1024, "".to_string()).is_err());
    }

    #[test]
    fn test_task_expiry() {
        init_test_config();
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .expires_in_hours(1)
            .build()
            .unwrap();

        assert!(!task.is_expired());
        // 由于时间计算的精度问题，允许有小的误差
        assert!(task.get_remaining_hours() >= 0);
    }

    #[test]
    fn test_extend_expiry() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .expires_in_hours(1)
            .build()
            .unwrap();

        let original_expiry = task.expires_at;

        // Valid extension
        assert!(task.extend_expiry(2).is_ok());
        assert!(task.expires_at > original_expiry);

        // Invalid extension (too long)
        assert!(task.extend_expiry(200).is_err());

        // Invalid extension (negative)
        assert!(task.extend_expiry(-1).is_err());
    }

    #[test]
    fn test_cancel_task() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Can cancel pending task
        assert!(task.cancel().is_ok());
        assert!(matches!(task.status, TaskStatus::Cancelled { .. }));

        // Cannot cancel completed task
        let mut completed_task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();
        completed_task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        assert!(completed_task.cancel().is_err());
    }

    #[test]
    fn test_reset_task() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Set to failed state
        task.set_error("Test error".to_string()).unwrap();
        task.progress = 50;

        // Reset should work
        assert!(task.reset().is_ok());
        assert!(matches!(task.status, TaskStatus::Pending { .. }));
        assert_eq!(task.progress, 0);
        assert!(task.error_message.is_none());
    }

    #[test]
    fn test_source_type_validation() {
        init_test_config();
        // Valid file upload path
        assert!(
            SourceType::Upload
                .validate_source_path(&Some("/path/to/file".to_string()))
                .is_ok()
        );

        // Empty file upload path should be ok (optional)
        assert!(SourceType::Upload.validate_source_path(&None).is_ok());

        // Valid URL
        assert!(
            SourceType::Url
                .validate_source_path(&Some("https://example.com/file.pdf".to_string()))
                .is_ok()
        );

        // Invalid URL (missing protocol)
        assert!(
            SourceType::Url
                .validate_source_path(&Some("example.com/file.pdf".to_string()))
                .is_err()
        );

        // Missing URL for download
        assert!(SourceType::Url.validate_source_path(&None).is_err());

        // Valid API path
        assert!(
            SourceType::ExternalApi
                .validate_source_path(&Some("/api/documents/123".to_string()))
                .is_ok()
        );
    }

    #[test]
    fn test_get_status_description() {
        init_test_config();
        let mut task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // 使用静态描述方法，避免时间相关的动态描述
        assert_eq!(task.status.get_static_description(), "等待处理");

        task.status = TaskStatus::Processing {
            stage: ProcessingStage::FormatDetection,
            started_at: Utc::now(),
            progress_details: None,
        };
        assert!(task.status.get_static_description().contains("处理中"));

        task.status = TaskStatus::new_completed(std::time::Duration::from_secs(60));
        assert_eq!(task.status.get_static_description(), "处理完成");

        task.status = TaskStatus::Failed {
            error: TaskError::new(
                "E001".to_string(),
                "Test error".to_string(),
                None,
            ),
            failed_at: Utc::now(),
            retry_count: 0,
            is_recoverable: false,
        };
        assert!(task.status.get_static_description().contains("处理失败"));

        task.status = TaskStatus::new_cancelled(Some("测试取消".to_string()));
        assert_eq!(task.status.get_static_description(), "已取消");
    }

    #[test]
    fn test_get_age_hours() {
        init_test_config();
        let task = DocumentTask::builder()
            .generate_id()
            .source_type(SourceType::Upload)
            .document_format(DocumentFormat::PDF)
            .build()
            .unwrap();

        // Should be 0 hours old (just created)
        assert_eq!(task.get_age_hours(), 0);
    }

    #[test]
    fn test_backward_compatibility() {
        init_test_config();
        // Test that the old constructor still works
        let task = DocumentTask::new(
            Uuid::new_v4().to_string(),
            SourceType::Upload,
            Some("/path/to/file.pdf".to_string()),
            DocumentFormat::PDF,
        );

        assert_eq!(task.source_type, SourceType::Upload);
        assert_eq!(task.document_format, DocumentFormat::PDF);
        assert_eq!(task.parser_engine, ParserEngine::MinerU);
        assert!(matches!(task.status, TaskStatus::Pending { .. }));
    }
}
