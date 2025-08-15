use std::sync::Arc;
use sled::Db;
use uuid::{Uuid, Timestamp, NoContext};
use crate::error::AppError;
use crate::models::{
    DocumentTask, SourceType, DocumentFormat, TaskStatus, ProcessingStage, ParserEngine
};

/// 任务服务
pub struct TaskService {
    db: Arc<Db>,
    tasks_tree: sled::Tree,
}

impl TaskService {
    /// 创建新的任务服务
    pub fn new(db: Arc<Db>) -> Result<Self, AppError> {
        let tasks_tree = db.open_tree("tasks")
            .map_err(|e| AppError::Database(format!("打开任务树失败: {}", e)))?;
        
        Ok(Self {
            db,
            tasks_tree,
        })
    }

    /// 创建新任务
    pub async fn create_task(
        &self,
        source_type: SourceType,
        source_path: Option<String>,
        format: DocumentFormat,
    ) -> Result<DocumentTask, AppError> {
        let task_id = Uuid::new_v7(Timestamp::now(NoContext)).to_string();
        
        log::info!("创建新任务: {} ({:?} -> {:?})", task_id, source_type, format);
        
        //根据 DocumentFormat ,如果PDF格式,使用 MinerU ,否则使用 MarkItDown
        let parser_engine = if format == DocumentFormat::PDF {
            ParserEngine::MinerU
        } else {
            ParserEngine::MarkItDown
        };

        let task = DocumentTask::builder()
            .id(task_id.clone())
            .source_type(source_type)
            .source_path(source_path)
            .document_format(format)
            .parser_engine(parser_engine) 
            .backend("pipeline")
            // file_size 和 mime_type 初始为 None，后续会更新
            .max_retries(3)
            .expires_in_hours(24)
            .build()?;
        
        // 保存到数据库
        self.save_task(&task).await?;
        
        Ok(task)
    }

    /// 获取任务
    pub async fn get_task(&self, task_id: &str) -> Result<Option<DocumentTask>, AppError> {
        log::debug!("查询任务: {}", task_id);
        
        match self.tasks_tree.get(task_id) {
            Ok(Some(data)) => {
                let task: DocumentTask = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Database(format!("反序列化任务失败: {}", e)))?;
                Ok(Some(task))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(AppError::Database(format!("查询任务失败: {}", e))),
        }
    }

    /// 保存任务
    pub async fn save_task(&self, task: &DocumentTask) -> Result<(), AppError> {
        let data = serde_json::to_vec(task)
            .map_err(|e| AppError::Database(format!("序列化任务失败: {}", e)))?;
        
        self.tasks_tree.insert(&task.id, data)
            .map_err(|e| AppError::Database(format!("保存任务失败: {}", e)))?;
        
        self.tasks_tree.flush()
            .map_err(|e| AppError::Database(format!("刷新数据库失败: {}", e)))?;
        
        log::debug!("任务已保存: {}", task.id);
        Ok(())
    }

    /// 更新任务状态
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), AppError> {
        log::info!("更新任务状态: {} -> {:?}", task_id, status);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        task.update_status(status);
        self.save_task(&task).await?;
        
        Ok(())
    }

    /// 更新任务处理阶段
    pub async fn update_task_stage(
        &self,
        task_id: &str,
        stage: ProcessingStage,
    ) -> Result<(), AppError> {
        log::info!("更新任务阶段: {} -> {:?}", task_id, stage);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        let _ = task.update_status(TaskStatus::new_processing(stage));
        self.save_task(&task).await?;
        
        Ok(())
    }

    /// 更新任务进度
    pub async fn update_task_progress(
        &self,
        task_id: &str,
        progress: u32,
    ) -> Result<(), AppError> {
        log::debug!("更新任务进度: {} -> {}%", task_id, progress);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        task.update_progress(progress);
        self.save_task(&task).await?;
        
        Ok(())
    }

    /// 设置任务错误
    pub async fn set_task_error(
        &self,
        task_id: &str,
        error_message: String,
    ) -> Result<(), AppError> {
        log::error!("任务错误: {} -> {}", task_id, error_message);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        task.set_error(error_message);
        self.save_task(&task).await?;
        
        Ok(())
    }

    /// 设置任务解析引擎
    pub async fn set_task_parser_engine(
        &self,
        task_id: &str,
        engine: ParserEngine,
    ) -> Result<(), AppError> {
        log::info!("设置任务解析引擎: {} -> {:?}", task_id, engine);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        task.parser_engine = engine;
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
        log::debug!("设置任务文件信息: {} (大小: {:?}, 类型: {:?})", task_id, file_size, mime_type);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
        if let (Some(size), Some(mime)) = (file_size, mime_type.clone()) {
            task.set_file_info(size, mime);
        } else {
            if let Some(size) = file_size { task.file_size = Some(size); }
            if let Some(mime) = mime_type { task.mime_type = Some(mime); }
        }
        
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
                Ok((_, data)) => {
                    match serde_json::from_slice::<DocumentTask>(&data) {
                        Ok(task) => {
                            tasks.push(task);
                            count += 1;
                        }
                        Err(e) => {
                            log::warn!("反序列化任务失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("读取任务数据失败: {}", e);
                }
            }
        }
        
        // 按创建时间倒序排列
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        
        Ok(tasks)
    }

    /// 取消任务
    pub async fn cancel_task(&self, task_id: &str, reason: Option<String>) -> Result<DocumentTask, AppError> {
        log::info!("取消任务: {} (原因: {:?})", task_id, reason);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
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
        log::info!("删除任务: {}", task_id);
        
        match self.tasks_tree.remove(task_id) {
            Ok(Some(_)) => {
                self.tasks_tree.flush()
                    .map_err(|e| AppError::Database(format!("刷新数据库失败: {}", e)))?;
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(AppError::Database(format!("删除任务失败: {}", e))),
        }
    }

    /// 重试任务
    pub async fn retry_task(&self, task_id: &str) -> Result<DocumentTask, AppError> {
        log::info!("重试任务: {}", task_id);
        
        let mut task = self.get_task(task_id).await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {}", task_id)))?;
        
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
                            log::warn!("反序列化任务失败: {}", e);
                            // 损坏的数据也删除
                            to_remove.push(key);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("读取任务数据失败: {}", e);
                }
            }
        }
        
        // 删除过期任务
        for key in to_remove {
            if let Err(e) = self.tasks_tree.remove(&key) {
                log::warn!("删除过期任务失败: {}", e);
            } else {
                cleaned_count += 1;
            }
        }
        
        if cleaned_count > 0 {
            self.tasks_tree.flush()
                .map_err(|e| AppError::Database(format!("刷新数据库失败: {}", e)))?;
            
            log::info!("清理了 {} 个过期任务", cleaned_count);
        }
        
        Ok(cleaned_count)
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
                            
                            match task.status {
                                TaskStatus::Pending { .. } => stats.pending_count += 1,
                                TaskStatus::Processing { .. } => stats.processing_count += 1,
                                TaskStatus::Completed { .. } => stats.completed_count += 1,
                                TaskStatus::Failed { .. } => stats.failed_count += 1,
                                TaskStatus::Cancelled { .. } => stats.cancelled_count += 1,
                            }
                            
                            match task.parser_engine {
                                ParserEngine::MinerU => stats.mineru_count += 1,
                                ParserEngine::MarkItDown => stats.markitdown_count += 1,
                            }
                        }
                        Err(e) => {
                            log::warn!("反序列化任务失败: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("读取任务数据失败: {}", e);
                }
            }
        }
        
        Ok(stats)
    }
}

/// 任务统计信息
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TaskStats {
    pub total_count: usize,
    pub pending_count: usize,
    pub processing_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
    pub cancelled_count: usize,
    pub mineru_count: usize,
    pub markitdown_count: usize,
}
