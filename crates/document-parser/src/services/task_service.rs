use crate::error::AppError;
use crate::models::{
    CreateTaskParams, DocumentFormat, DocumentTask, ParserEngine, ProcessingStage, SourceType,
    TaskError, TaskStatus,
};
use sled::Db;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::{NoContext, Timestamp, Uuid};

/// 任务服务
pub struct TaskService {
    tasks_tree: sled::Tree,
    /// 任务记录写串行锁：get_task → 改 → save_task 的读改写窗口内并发写会互相
    /// 覆盖（30s 心跳 touch 可回滚刚写入的 Cancelled）。所有改状态的 RMW 方法
    /// 全程持锁；sled 自身线程安全，读路径不持锁
    write_lock: tokio::sync::Mutex<()>,
}

impl TaskService {
    /// 创建新的任务服务
    pub fn new(db: Arc<Db>) -> Result<Self, AppError> {
        let tasks_tree = db
            .open_tree("tasks")
            .map_err(|e| AppError::Database(format!("打开任务树失败: {e}")))?;

        Ok(Self {
            tasks_tree,
            write_lock: tokio::sync::Mutex::new(()),
        })
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
            "Create new task: {} ({:?} -> {:?})",
            task_id, source_type, format
        );

        let task = DocumentTask::new(CreateTaskParams {
            id: task_id.clone(),
            source_type: source_type.clone(),
            source: source_path,
            original_filename,
            document_format: format,
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });

        // 保存到数据库
        self.save_task(&task).await?;

        Ok(task)
    }

    /// 获取任务
    pub async fn get_task(&self, task_id: &str) -> Result<Option<DocumentTask>, AppError> {
        debug!("Query task: {}", task_id);

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

        debug!("Task saved: {}", task.id);
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
        debug!("Update basic task information: {}", task_id);

        let _guard = self.write_lock.lock().await;
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
    ///
    /// 终态保护：Cancelled/Completed 是单向终态——非终态不得回写（用户取消后
    /// worker 的阶段推进会把状态打回 Processing），不同终态变体间也不得翻写
    /// （worker 对已取消任务写 Completed 会抹掉取消事实）；同变体幂等重写放行。
    /// 需要真正覆盖终态的场景（仅服务重启恢复）用 [`Self::force_update_task_status`]。
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), AppError> {
        info!("Update task status: {} -> {:?}", task_id, status);

        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        if is_terminal(&task.status)
            && std::mem::discriminant(&task.status) != std::mem::discriminant(&status)
        {
            warn!(
                "update_task_status skipped: task {} terminal {:?} 不被翻写为 {:?}",
                task_id, task.status, status
            );
            return Ok(());
        }

        task.update_status(status)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 强制更新任务状态（跳过终态保护；仅服务重启的 restore 路径使用）
    pub async fn force_update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), AppError> {
        info!("Force update task status: {} -> {:?}", task_id, status);

        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.update_status(status)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 更新任务处理阶段（终态保护同 [`Self::update_task_status`]——取消后的
    /// 阶段推进不打回 Processing）
    pub async fn update_task_stage(
        &self,
        task_id: &str,
        stage: ProcessingStage,
    ) -> Result<(), AppError> {
        info!("Update task stage: {} -> {:?}", task_id, stage);

        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        if is_terminal(&task.status) {
            warn!(
                "update_task_stage skipped: task {} already terminal {:?}",
                task_id, task.status
            );
            return Ok(());
        }

        let _ = task.update_status(TaskStatus::new_processing(stage));
        self.save_task(&task).await?;

        Ok(())
    }

    /// 更新任务进度
    ///
    /// 持写锁：解析期进度回调与 cancel_task 并发时，整条覆盖同样会把刚写入的
    /// Cancelled 回滚（与 touch_task 同类的 RMW 竞态）
    pub async fn update_task_progress(&self, task_id: &str, progress: u32) -> Result<(), AppError> {
        debug!("Update task progress: {} -> {}%", task_id, progress);

        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.update_progress(progress)?;
        self.save_task(&task).await?;

        Ok(())
    }

    /// 设置任务错误
    ///
    /// 注意：本方法经 `update_status` 会**自增 retry_count**（消耗重试额度）。
    /// 任务入队**前**的失败路径（handler 早退中止）请改用 [`Self::abort_task`]
    /// （显式传当前值，不消耗额度）。
    pub async fn set_task_error(
        &self,
        task_id: &str,
        error_message: String,
    ) -> Result<(), AppError> {
        error!("Task error: {} -> {}", task_id, error_message);

        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        // 终态保护：Cancelled（用户取消后子进程被 kill，解析返回的取消错误会
        // 走到这里）与 Completed 不被晚到的失败覆盖——状态机单向性
        if is_terminal(&task.status) {
            warn!(
                "set_task_error skipped: task {} already in terminal state {:?}",
                task_id,
                std::mem::discriminant(&task.status)
            );
            return Ok(());
        }

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
        info!("Set task parsing engine: {} -> {:?}", task_id, engine);

        let _guard = self.write_lock.lock().await;
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
            "Set task file information: {} (size: {:?}, type: {:?})",
            task_id, file_size, mime_type
        );

        let _guard = self.write_lock.lock().await;
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
            "Update task source information: task_id={}, path={:?}, url={:?}, filename={:?}",
            task_id, source_path, source_url, original_filename
        );

        let _guard = self.write_lock.lock().await;
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
        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.bucket_dir = bucket_dir;
        task.updated_at = chrono::Utc::now();

        self.save_task(&task).await?;
        Ok(())
    }

    /// 中止任务（置 Failed 但**不消耗重试额度**）
    ///
    /// 与 [`Self::set_task_error`] 的区别：set_task_error 经 `update_status`
    /// 会对 Failed 自增 `retry_count`——对从未执行过的任务（handler 在入队前
    /// 的失败路径）调用会白白烧掉一次重试额度。本方法显式传入当前值。
    /// 自身失败会记录 error 日志（调用方无需再吞）。
    pub async fn abort_task(&self, task_id: &str, message: String) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        let mut task = match self.get_task(task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                error!("Abort task failed, task not found: {task_id}");
                return Err(AppError::Task(format!("任务不存在: {task_id}")));
            }
            Err(e) => {
                error!("Abort task failed, read task error: {task_id} -> {e}");
                return Err(e);
            }
        };

        // 与 set_error 的双出口语义对齐：顶层 error_message 供任务列表接口
        // 直接读取（task_handler 取 task.error_message），缺失会显示 null
        task.error_message = Some(message.clone());
        let task_error = TaskError::new(
            "E010".to_string(),
            message.clone(),
            task.status.get_current_stage().cloned(),
        );
        // 显式传当前 retry_count，绕过 update_status 对 Failed 的自增副作用
        task.status = TaskStatus::new_failed(task_error, task.retry_count);
        task.updated_at = chrono::Utc::now();

        if let Err(e) = self.save_task(&task).await {
            error!("Abort task failed, save task error: {task_id} -> {e}");
            return Err(e);
        }
        error!("Task aborted before enqueue: {task_id} -> {message}");
        Ok(())
    }

    /// 设置任务的自定义上传端点（Some=自定义后端，None=OSS）
    ///
    /// 必须在任务入队前调用：worker 只携带 task_id，上传配置只能从任务读取。
    pub async fn set_task_upload_config(
        &self,
        task_id: &str,
        upload_config: Option<crate::models::UploadEndpoint>,
    ) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

        task.upload_config = upload_config;
        task.updated_at = chrono::Utc::now();

        self.save_task(&task).await?;
        Ok(())
    }

    /// 列出所有任务
    pub async fn list_tasks(&self, limit: Option<usize>) -> Result<Vec<DocumentTask>, AppError> {
        let mut tasks = Vec::new();
        let mut count = 0;

        for result in self.tasks_tree.iter() {
            if let Some(max_count) = limit
                && count >= max_count
            {
                break;
            }

            match result {
                Ok((_, data)) => match serde_json::from_slice::<DocumentTask>(&data) {
                    Ok(task) => {
                        tasks.push(task);
                        count += 1;
                    }
                    Err(e) => {
                        warn!("Deserialization task failed: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read task data: {}", e);
                }
            }
        }

        // 按创建时间倒序排列
        tasks.sort_by_key(|a| std::cmp::Reverse(a.created_at));

        Ok(tasks)
    }

    /// 取消任务
    ///
    /// 状态改为 Cancelled 后触发解析取消令牌（若任务正在解析）：execute 层
    /// select 轮询到令牌即 kill 解析子进程（此前取消只改状态，子进程照跑）。
    /// 令牌触发在写锁外（持锁跨 await 会放大锁竞争，且无必要）。
    pub async fn cancel_task(
        &self,
        task_id: &str,
        reason: Option<String>,
    ) -> Result<DocumentTask, AppError> {
        info!("Cancel task: {} (reason: {:?})", task_id, reason);

        let task = {
            let _guard = self.write_lock.lock().await;
            let mut task = self
                .get_task(task_id)
                .await?
                .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;

            // 使用任务模型的 cancel 方法（终态任务拒绝取消）
            task.cancel()?;

            // 如果提供了原因，更新取消状态
            if let Some(cancel_reason) = reason {
                task.status = TaskStatus::new_cancelled(Some(cancel_reason));
            }

            self.save_task(&task).await?;
            task
        };

        // 通知解析层中止（令牌由 DocumentService 在解析开始时注册；任务不在
        // 解析中时无令牌，静默跳过）
        if crate::services::parse_cancel::request_parse_cancel(task_id).await {
            info!("Parse cancel signalled for task: {}", task_id);
        }

        Ok(task)
    }

    /// 心跳 touch：仅推进 updated_at（不改动状态/进度），由 DocumentService
    /// 解析期 30s 周期调用——让"长解析"与"卡死"在运维视角可区分。
    /// 持写锁：touch 的读改写窗口若与 cancel_task 并发，整条覆盖会把刚写入的
    /// Cancelled 回滚成旧状态
    pub async fn touch_task(&self, task_id: &str) -> Result<(), AppError> {
        let _guard = self.write_lock.lock().await;
        let mut task = self
            .get_task(task_id)
            .await?
            .ok_or_else(|| AppError::Task(format!("任务不存在: {task_id}")))?;
        task.touch();
        self.save_task(&task).await
    }

    /// 删除任务
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, AppError> {
        info!("Delete task: {}", task_id);

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
        info!("Retry task: {}", task_id);

        let _guard = self.write_lock.lock().await;
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
                            warn!("Deserialization task failed: {}", e);
                            // 损坏的数据也删除
                            to_remove.push(key);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read task data: {}", e);
                }
            }
        }

        // 删除过期任务并清理相关文件
        for key in to_remove {
            // 获取任务信息以便清理文件
            if let Ok(data) = self.tasks_tree.get(&key)
                && let Some(data) = data
                && let Ok(task) = serde_json::from_slice::<DocumentTask>(&data)
            {
                // 清理任务相关的临时文件
                self.cleanup_task_files(&task).await;
            }

            if let Err(e) = self.tasks_tree.remove(&key) {
                warn!("Failed to delete expired tasks: {}", e);
            } else {
                cleaned_count += 1;
            }
        }

        if cleaned_count > 0 {
            self.tasks_tree
                .flush()
                .map_err(|e| AppError::Database(format!("刷新数据库失败: {e}")))?;

            info!("Cleaned up {} expired tasks", cleaned_count);
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
                        "Cleanup task {}'s temporary files failed: {} - {}",
                        task.id, source_path, e
                    );
                } else {
                    info!(
                        "Cleaned temporary files of task {}: {}",
                        task.id, source_path
                    );
                }
            }
        }

        // 清理可能的工作目录
        let temp_dir = std::env::temp_dir();
        let task_work_dir = temp_dir.join(format!("document_parser_{}", task.id));
        if task_work_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&task_work_dir).await {
                warn!(
                    "Cleanup task {}'s working directory failed: {} - {}",
                    task.id,
                    task_work_dir.display(),
                    e
                );
            } else {
                info!(
                    "Cleaned working directory of task {}: {}",
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
                            warn!("Deserialization task failed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read task data: {}", e);
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

/// 是否处于单向终态（Cancelled / Completed）—— Failed 允许重试/取消，不算
fn is_terminal(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Cancelled { .. } | TaskStatus::Completed { .. }
    )
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

#[cfg(test)]
mod heartbeat_cancel_tests {
    use super::*;
    use crate::models::SourceType;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn setup() -> (TempDir, TaskService) {
        let dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(dir.path()).unwrap());
        let svc = TaskService::new(db).unwrap();
        (dir, svc)
    }

    async fn create_pending_task(svc: &TaskService) -> String {
        let task = svc
            .create_task(
                SourceType::Upload,
                Some("/tmp/x.md".to_string()),
                Some("x.md".to_string()),
                None,
            )
            .await
            .unwrap();
        task.id
    }

    /// 终态保护：Cancelled 不被晚到的 set_task_error 覆盖
    ///（取消 → 子进程被 kill → 解析返回取消错误 → worker 走 set_task_error 的防线）
    #[tokio::test]
    async fn set_task_error_does_not_overwrite_cancelled() {
        let (_dir, svc) = setup().await;
        let id = create_pending_task(&svc).await;

        svc.cancel_task(&id, Some("用户取消".to_string()))
            .await
            .unwrap();
        svc.set_task_error(&id, "解析已取消（子进程被 kill 的迟到错误）".to_string())
            .await
            .unwrap();

        let task = svc.get_task(&id).await.unwrap().unwrap();
        assert!(
            matches!(task.status, TaskStatus::Cancelled { .. }),
            "Cancelled 终态不应被 set_task_error 覆盖，实际 {:?}",
            task.status
        );
    }

    /// 终态变体间不得翻写：Cancelled 不能被 worker 的 Completed 抹掉取消事实
    ///（排队期取消的任务被 worker 捞到后的收尾写入防线）
    #[tokio::test]
    async fn update_task_status_cannot_flip_cancelled_to_completed() {
        let (_dir, svc) = setup().await;
        let id = create_pending_task(&svc).await;

        svc.cancel_task(&id, Some("用户取消".to_string()))
            .await
            .unwrap();

        svc.update_task_status(
            &id,
            TaskStatus::new_completed(std::time::Duration::from_secs(1)),
        )
        .await
        .unwrap();

        let task = svc.get_task(&id).await.unwrap().unwrap();
        assert!(
            matches!(task.status, TaskStatus::Cancelled { .. }),
            "Cancelled 不应被翻写为 Completed，实际 {:?}",
            task.status
        );

        // 同变体幂等重写放行（如取消原因更新）
        let task = svc.get_task(&id).await.unwrap().unwrap();
        svc.update_task_status(&id, task.status).await.unwrap();
    }

    /// 心跳 touch 不回滚 Cancelled（写锁下 RMW 原子化）
    #[tokio::test]
    async fn touch_task_keeps_cancelled_status() {
        let (_dir, svc) = setup().await;
        let id = create_pending_task(&svc).await;

        svc.cancel_task(&id, None).await.unwrap();
        svc.touch_task(&id).await.unwrap();

        let task = svc.get_task(&id).await.unwrap().unwrap();
        assert!(
            matches!(task.status, TaskStatus::Cancelled { .. }),
            "touch 不应回滚 Cancelled，实际 {:?}",
            task.status
        );
    }

    /// 心跳 touch：updated_at 推进（手动回拨时间戳模拟长解析后 touch）
    #[tokio::test]
    async fn touch_task_advances_updated_at() {
        let (_dir, svc) = setup().await;
        let id = create_pending_task(&svc).await;

        // 回拨 updated_at 一小时（模拟 MinerUExecuting 长阶段、无任何任务写入）
        {
            let mut task = svc.get_task(&id).await.unwrap().unwrap();
            task.updated_at = chrono::Utc::now() - chrono::Duration::hours(1);
            svc.save_task(&task).await.unwrap();
        }

        svc.touch_task(&id).await.unwrap();
        let task = svc.get_task(&id).await.unwrap().unwrap();
        let age = chrono::Utc::now().signed_duration_since(task.updated_at);
        assert!(
            age < chrono::Duration::seconds(5),
            "touch 后 updated_at 应推进到当前，实际距今 {age}"
        );
    }

    /// cancel_task 触发解析取消令牌（注册表联动）
    #[tokio::test]
    async fn cancel_task_signals_registered_parse_token() {
        crate::services::parse_cancel::clear_for_test();
        let (_dir, svc) = setup().await;
        let id = create_pending_task(&svc).await;

        // 模拟 DocumentService 解析开始时的注册
        let token = crate::parsers::mineru_parser::CancellationToken::new();
        crate::services::parse_cancel::register(&id, token.clone());

        svc.cancel_task(&id, None).await.unwrap();
        assert!(
            token.is_cancelled().await,
            "cancel_task 应触发已注册的解析取消令牌"
        );

        let task = svc.get_task(&id).await.unwrap().unwrap();
        assert!(matches!(task.status, TaskStatus::Cancelled { .. }));
        crate::services::parse_cancel::unregister(&id);
    }
}
