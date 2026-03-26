use crate::error::AppError;
use crate::models::{
    DocumentFormat, DocumentTask, ParserEngine, ProcessingStage, SourceType, TaskStatus,
};
use sled::Db;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::{NoContext, Timestamp, Uuid};

/// 任务服务
pub struct TaskService {
    tasks_tree: sled::Tree,
}

impl TaskService {
    /// 创建新的任务服务
    pub fn new(db: Arc<Db>) -> Result<Self, AppError> {
        let tasks_tree = db
            .open_tree("tasks")
            .map_err(|e| AppError::Database(format!("打开任务树失败: {e}")))?;

        Ok(Self { tasks_tree })
    }

    /// 创建新任务
    pub async fn create_task(
        &self,
        source_type: SourceType,
        source_path: Option<String>,
        original_filename: Option<String>,
        format: Option<DocumentFormat>,
    ) -> Result<DocumentTask, AppError> {
        let task_id = Uuid::new_v7(Timestamp::now(NoContext)).to_string();

        info!(
            "创建新任务: {} ({:?} -> {:?})",
            task_id, source_type, format
        );

        let task = DocumentTask::new(
            task_id.clone(),
            source_type.clone(),
            source_path,
            original_filename,
            format,
            Some("pipeline".to_string()),
            Some(24),
            Some(3),
        );

        // 保存到数据库
        self.save_task(&task).await?;

        Ok(task)
    }

    /// 获取任务
    pub async fn get_task(&self, task_id: &str) -> Result<Option<DocumentTask>, AppError> {
        debug!("查询任务: {}", task_id);

        match self.tasks_tree.get(task_id) {
            Ok(Some(data)) => {
                let task: DocumentTask = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Database(format!("反序列化任务失败: {e}")))?;
                Ok(Some(task))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(AppError::Database(format!("查询任务失败: {e}"))),
        }
    }

    /// 保存任务
    pub async fn save_task(&self, task: &DocumentTask) -> Result<(), AppError> {
        let data = serde_json::to_vec(task)
            .map_err(|e| AppError::Database(format!("序列化任务失败: {e}")))?;

        self.tasks_tree
            .insert(&task.id, data)
            .map_err(|e| AppError::Database(format!("保存任务失败: {e}")))?;

        self.tasks_tree
            .flush()
            .map_err(|e| AppError::Database(format!("刷新数据库失败: {e}")))?;

        debug!("任务已保存: {}", task.id);
        Ok(())
    }

    /// 更新任务基本信息
    pub async fn update_task(
        &self,
        task_id: &str,
        source_path: Option<String>,
        original_filename: Option<String>,
        document_format: DocumentFormat,
    ) -> Result<(), AppError> {
        debug!("更新任务基本信息: {}", task_id);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        // 更新任务信息
        if let Some(path) = source_path {
            task.source_path = Some(path);
        }
        if let Some(filename) = original_filename {
            task.original_filename = Some(filename);
        }
        // 根据文档格式更新解析引擎
        task.parser_engine = Some(if document_format == DocumentFormat::PDF {
            ParserEngine::MinerU
        } else {
            ParserEngine::MarkItDown
        });

        task.document_format = Some(document_format);

        // 更新时间戳
        task.updated_at = chrono::Utc::now();

        self.save_task(&task).await?;
        Ok(())
    }

    /// 更新任务状态
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), AppError> {
        info!("更新任务状态: {} -> {:?}", task_id, status);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.update_status(status)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 更新任务处理阶段
    pub async fn update_task_stage(
        &self,
        task_id: &str,
        stage: ProcessingStage,
    ) -> Result<(), AppError> {
        info!("更新任务阶段: {} -> {:?}", task_id, stage);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        let _ = task.update_status(TaskStatus::new_processing(stage));
        self.save_task(&task).await?;

        Ok(())
    }

    /// 更新任务进度
    pub async fn update_task_progress(&self, task_id: &str, progress: u32) -> Result<(), AppError> {
        debug!("更新任务进度: {} -> {}%", task_id, progress);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.update_progress(progress)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 设置任务错误
    pub async fn set_task_error(
        &self,
        task_id: &str,
        error_message: String,
    ) -> Result<(), AppError> {
        error!("任务错误: {} -> {}", task_id, error_message);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.set_error(error_message)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 设置任务解析引擎
    pub async fn set_task_parser_engine(
        &self,
        task_id: &str,
        engine: ParserEngine,
    ) -> Result<(), AppError> {
        info!("设置任务解析引擎: {} -> {:?}", task_id, engine);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.parser_engine = Some(engine);
        self.save_task(&task).await?;

        Ok(())
    }

    /// 设置任务文件信息
    pub async fn set_task_file_info(
        &self,
        task_id: &str,
        file_size: Option<u64>,
        mime_type: Option<String>,
    ) -> Result<(), AppError> {
        debug!(
            "设置任务文件信息: {} (大小: {:?}, 类型: {:?})",
            task_id, file_size, mime_type
        );

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        if let (Some(size), Some(mime)) = (file_size, mime_type.clone()) {
            task.set_file_info(size, mime)?;
        } else {
            if let Some(size) = file_size {
                task.file_size = Some(size);
            }
            if let Some(mime) = mime_type {
                task.mime_type = Some(mime);
            }
        }

        self.save_task(&task).await?;
        Ok(())
    }

    /// 更新任务的来源信息（本地路径、URL、原始文件名）
    pub async fn update_task_source_info(
        &self,
        task_id: &str,
        source_path: Option<String>,
        source_url: Option<String>,
        original_filename: Option<String>,
    ) -> Result<(), AppError> {
        debug!(
            "更新任务来源信息: task_id={}, path={:?}, url={:?}, filename={:?}",
            task_id, source_path, source_url, original_filename
        );

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        if let Some(path) = source_path {
            task.source_path = Some(path);
        }
        if let Some(url) = source_url {
            task.source_url = Some(url);
        }
        if let Some(name) = original_filename {
            task.original_filename = Some(name);
        }

        task.updated_at = chrono::Utc::now();

        self.save_task(&task).await?;
        Ok(())
    }

    /// 设置任务的 OSS 子目录（bucket_dir）
    pub async fn set_task_bucket_dir(
        &self,
        task_id: &str,
        bucket_dir: Option<String>,
    ) -> Result<(), AppError> {
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.bucket_dir = bucket_dir;
        task.updated_at = chrono::Utc::now();

        self.save_task(&task).await?;
        Ok(())
    }

    /// 列出所有任务
    pub async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<DocumentTask>, AppError> {
        let mut tasks = Vec::new();
        let mut count = 0;

        for result in self.tasks_tree.iter() {
            if let Some(max_count) = limit {
                if count >= max_count {
                    break;
                }
            }

            match result {
                Ok((_, data)) => match serde_json::from_slice::<DocumentTask>(&data) {
                    Ok(task) => {
                        tasks.push(task);
                        count += 1;
                    }
                    Err(e) => {
                        warn!("反序列化任务失败: {}", e);
                    }
                },
                Err(e) => {
                    warn!("读取任务数据失败: {}", e);
                }
            }
        }

        // 按创建时间倒序排列
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        Ok(tasks)
    }

    /// 取消任务
    pub async fn cancel_task(
        &self,
        task_id: &str,
        reason: Option<String>,
    ) -> Result<DocumentTask, AppError> {
        info!("取消任务: {} (原因: {:?})", task_id, reason);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        // 使用任务模型的 cancel 方法
        task.cancel()?;

        // 如果提供了原因，更新取消状态
        if let Some(cancel_reason) = reason {
            task.status = TaskStatus::new_cancelled(Some(cancel_reason));
        }

        self.save_task(&task).await?;

        Ok(task)
    }

    /// 删除任务
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, AppError> {
        info!("删除任务: {}", task_id);

        // 获取任务信息以便清理相关文件
        let task = self.get_task(task_id).await?;

        match self.tasks_tree.remove(task_id) {
            Ok(Some(_)) => {
                self.tasks_tree
                    .flush()
                    .map_err(|e| AppError::Database(format!("刷新数据库失败: {e}")))?;

                // 清理任务相关的临时文件
                if let Some(task) = task {
                    self.cleanup_task_files(&task).await;
                }

                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(AppError::Database(format!("删除任务失败: {e}"))),
        }
    }

    /// 重试任务
    pub async fn retry_task(&self, task_id: &str) -> Result<DocumentTask, AppError> {
        info!("重试任务: {}", task_id);

        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        // 使用任务模型的 reset 方法
        task.reset()?;

        self.save_task(&task).await?;

        Ok(task)
    }

    /// 清理过期任务
    pub async fn cleanup_expired_tasks(&self) -> Result<usize, AppError> {
        let mut cleaned_count = 0;
        let mut to_remove = Vec::new();

        for result in self.tasks_tree.iter() {
            match result {
                Ok((key, data)) => {
                    match serde_json::from_slice::<DocumentTask>(&data) {
                        Ok(task) => {
                            if task.is_expired() {
                                to_remove.push(key);
                            }
                        }
                        Err(e) => {
                            warn!("反序列化任务失败: {}", e);
                            // 损坏的数据也删除
                            to_remove.push(key);
                        }
                    }
                }
                Err(e) => {
                    warn!("读取任务数据失败: {}", e);
                }
            }
        }

        // 删除过期任务并清理相关文件
        for key in to_remove {
            // 获取任务信息以便清理文件
            if let Ok(data) = self.tasks_tree.get(&key) {
                if let Some(data) = data {
                    if let Ok(task) = serde_json::from_slice::<DocumentTask>(&data) {
                        // 清理任务相关的临时文件
                        self.cleanup_task_files(&task).await;
                    }
                }
            }

            if let Err(e) = self.tasks_tree.remove(&key) {
                warn!("删除过期任务失败: {}", e);
            } else {
                cleaned_count += 1;
            }
        }

        if cleaned_count > 0 {
            self.tasks_tree
                .flush()
                .map_err(|e| AppError::Database(format!("刷新数据库失败: {e}")))?;

            info!("清理了 {} 个过期任务", cleaned_count);
        }

        Ok(cleaned_count)
    }

    /// 清理任务相关的临时文件
    async fn cleanup_task_files(&self, task: &DocumentTask) {
        // 清理基于 taskId 的临时文件
        if let Some(source_path) = &task.source_path {
            // 如果是基于 taskId 的文件路径，进行清理
            if source_path.contains(&task.id) {
                if let Err(e) = tokio::fs::remove_file(source_path).await {
                    warn!(
                        "清理任务 {} 的临时文件失败: {} - {}",
                        task.id, source_path, e
                    );
                } else {
                    info!("已清理任务 {} 的临时文件: {}", task.id, source_path);
                }
            }
        }

        // 清理可能的工作目录
        let temp_dir = std::env::temp_dir();
        let task_work_dir = temp_dir.join(format!("document_parser_{}", task.id));
        if task_work_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&task_work_dir).await {
                warn!(
                    "清理任务 {} 的工作目录失败: {} - {}",
                    task.id,
                    task_work_dir.display(),
                    e
                );
            } else {
                info!(
                    "已清理任务 {} 的工作目录: {}",
                    task.id,
                    task_work_dir.display()
                );
            }
        }
    }

    /// 获取任务统计信息
    pub async fn get_task_stats(&self) -> Result<TaskStats, AppError> {
        let mut stats = TaskStats::default();

        for result in self.tasks_tree.iter() {
            match result {
                Ok((_, data)) => {
                    match serde_json::from_slice::<DocumentTask>(&data) {
                        Ok(task) => {
                            stats.total_count += 1;
                            let id = task.id.clone();

                            match task.status {
                                TaskStatus::Pending { .. } => {
                                    stats.pending_count += 1;
                                    stats.pending_ids.push(id);
                                }
                                TaskStatus::Processing { .. } => {
                                    stats.processing_count += 1;
                                    stats.processing_ids.push(id);
                                }
                                TaskStatus::Completed {
                                    processing_time, ..
                                } => {
                                    stats.completed_count += 1;
                                    stats.completed_ids.push(id.clone());

                                    // 记录执行时间信息
                                    let processing_time_ms = processing_time.as_millis() as u64;
                                    stats.completed_task_times.push(CompletedTaskTime {
                                        task_id: id,
                                        processing_time_ms,
                                    });
                                }
                                TaskStatus::Failed { error, .. } => {
                                    stats.failed_count += 1;
                                    stats.failed_ids.push(id.clone());
                                    stats.failed_details.push(FailedTaskSummary {
                                        task_id: id,
                                        error_code: error.error_code,
                                        error_message: error.error_message,
                                        stage: error.stage,
                                    });
                                }
                                TaskStatus::Cancelled { .. } => {
                                    stats.cancelled_count += 1;
                                    stats.cancelled_ids.push(id);
                                }
                            }

                            if let Some(engine) = task.parser_engine {
                                match engine {
                                    ParserEngine::MinerU => stats.mineru_count += 1,
                                    ParserEngine::MarkItDown => stats.markitdown_count += 1,
                                }
                            }
                        }
                        Err(e) => {
                            warn!("反序列化任务失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("读取任务数据失败: {}", e);
                }
            }
        }

        // 计算已完成任务的平均执行时间
        if !stats.completed_task_times.is_empty() {
            let total_time_ms: u64 = stats
                .completed_task_times
                .iter()
                .map(|task_time| task_time.processing_time_ms)
                .sum();
            stats.average_processing_time_ms =
                Some(total_time_ms / stats.completed_task_times.len() as u64);
        }

        Ok(stats)
    }
}

/// 任务统计信息
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TaskStats {
    pub total_count: usize,
    pub pending_count: usize,
    pub processing_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub cancelled_count: usize,
    pub mineru_count: usize,
    pub markitdown_count: usize,
    /// 待处理任务ID列表
    pub pending_ids: Vec<String>,
    /// 处理中任务ID列表
    pub processing_ids: Vec<String>,
    /// 已完成任务ID列表
    pub completed_ids: Vec<String>,
    /// 已取消任务ID列表
    pub cancelled_ids: Vec<String>,
    /// 失败任务ID列表
    pub failed_ids: Vec<String>,
    /// 失败任务详情列表（包含错误码、错误信息与阶段）
    pub failed_details: Vec<FailedTaskSummary>,
    /// 已完成任务的执行时间详情列表
    pub completed_task_times: Vec<CompletedTaskTime>,
    /// 已完成任务的平均执行时间（毫秒）
    pub average_processing_time_ms: Option<u64>,
}

/// 失败任务简要信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct FailedTaskSummary {
    /// 任务ID
    pub task_id: String,
    /// 错误码（如 E009、E003）
    pub error_code: String,
    /// 错误信息
    pub error_message: String,
    /// 发生错误时的处理阶段
    pub stage: Option<ProcessingStage>,
}

/// 已完成任务执行时间信息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CompletedTaskTime {
    /// 任务ID
    pub task_id: String,
    /// 执行耗时（毫秒）
    pub processing_time_ms: u64,
}
