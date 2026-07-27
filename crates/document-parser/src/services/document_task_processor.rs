use std::sync::Arc;

use log::info;

use crate::error::AppError;
use crate::models::SourceType;

use super::{DocumentService, TaskProcessor, TaskService};

/// 文档任务处理器
///
/// 基于任务ID从任务服务获取任务详情，根据 `source_type` 分派到对应的解析逻辑。
pub struct DocumentTaskProcessor {
    document_service: Arc<DocumentService>,
    task_service: Arc<TaskService>,
}

impl DocumentTaskProcessor {
    /// 创建新的文档任务处理器
    pub fn new(document_service: Arc<DocumentService>, task_service: Arc<TaskService>) -> Self {
        Self {
            document_service,
            task_service,
        }
    }
}

#[async_trait::async_trait]
impl TaskProcessor for DocumentTaskProcessor {
    async fn process_task(&self, task_id: &str) -> Result<(), AppError> {
        // 获取任务
        let task = self
            .task_service
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        match task.source_type {
            SourceType::Upload => {
                let file_path = task
                    .source_path
                    .ok_or_else(|| AppError::Task("上传任务缺少文件路径".to_string()))?;

                // 使用文件路径解析
                // 由 DocumentService 负责更新状态、进度、结果等
                let _ = self
                    .document_service
                    .parse_document(&task.id, &file_path)
                    .await
                    .map_err(|e| AppError::Processing(e.to_string()))?;
                Ok(())
            }
            SourceType::Url => {
                // 优先使用新的 source_url 字段，兼容旧数据回退到 source_path
                let url = task
                    .source_url
                    .or(task.source_path.clone())
                    .ok_or_else(|| AppError::Task("URL 任务缺少下载地址".to_string()))?;

                info!("Start parsing URL task: {url}");
                let _ = self
                    .document_service
                    .parse_document_from_url(&task.id, &url)
                    .await
                    .map_err(|e| AppError::Processing(e.to_string()))?;
                Ok(())
            }
        }
    }
}
