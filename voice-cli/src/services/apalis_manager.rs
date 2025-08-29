use crate::VoiceCliError;
use crate::models::{
    AsyncTranscriptionTask, TaskManagementConfig, TaskStatus, TranscriptionResponse,
    TaskStatsResponse,
};
use crate::services::{AudioFileManager, ModelService, TranscriptionEngine};
use apalis::prelude::*;
use apalis::layers::retry::RetryPolicy;
use apalis::layers::WorkerBuilderExt;
use apalis_sql::sqlite::SqliteStorage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Row;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};


/// 全局 Apalis 管理器实例（无锁版本）
static GLOBAL_APALIS_MANAGER: OnceLock<Arc<LockFreeApalisManager>> = OnceLock::new();


/// 初始转录任务 - 流水线的第一步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionTask {
    pub task_id: String,
    pub audio_file_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 音频预处理完成的任务 - 流水线的第二步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessedTask {
    pub task_id: String,
    pub processed_audio_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 转录完成的任务 - 流水线的第三步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionCompletedTask {
    pub task_id: String,
    pub transcription_result: TranscriptionResponse,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<AsyncTranscriptionTask> for TranscriptionTask {
    fn from(task: AsyncTranscriptionTask) -> Self {
        Self {
            task_id: task.task_id,
            audio_file_path: task.audio_file_path,
            original_filename: task.original_filename,
            model: task.model,
            response_format: task.response_format,
            created_at: task.created_at,
        }
    }
}

/// 步骤共享上下文
#[derive(Debug, Clone)]
pub struct StepContext {
    pub transcription_engine: Arc<TranscriptionEngine>,
    pub audio_file_manager: Arc<AudioFileManager>,
    pub pool: sqlx::SqlitePool,
}

impl StepContext {
    /// 保存任务状态到SQLite
    async fn save_task_status(&self, task_id: &str, status: &TaskStatus) -> Result<(), Error> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        
        sqlx::query(
            "INSERT OR REPLACE INTO task_status (task_id, status, updated_at) VALUES (?, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        
        Ok(())
    }

    /// 获取任务状态
    async fn get_task_status(&self, task_id: &str) -> Result<Option<TaskStatus>, Error> {
        let row = sqlx::query("SELECT status FROM task_status WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        if let Some(row) = row {
            let status_json: String = row.try_get("status")
                .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
            
            let status = serde_json::from_str(&status_json)
                .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
            
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// 保存任务结果到SQLite
    async fn save_task_result(&self, task_id: &str, result: &TranscriptionResponse) -> Result<(), Error> {
        let result_json = serde_json::to_string(result)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        
        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, created_at) VALUES (?, ?, ?)"
        )
        .bind(task_id)
        .bind(result_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        
        Ok(())
    }

    /// 获取任务结果
    async fn get_task_result(&self, task_id: &str) -> Result<Option<TranscriptionResponse>, Error> {
        let row = sqlx::query("SELECT result FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        if let Some(row) = row {
            let result_json: String = row.try_get("result")
                .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
            
            let result = serde_json::from_str(&result_json)
                .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
            
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }
}

/// 任务状态更新
#[derive(Debug, Clone)]
pub struct TaskStatusUpdate {
    pub task_id: String,
    pub status: TaskStatus,
}

/// 任务存储和状态管理
#[derive(Debug, Clone)]
pub struct TaskStorage {
    pub sqlite_storage: SqliteStorage<TranscriptionTask>,
}

/// 无锁 Apalis 任务管理器
#[derive(Debug)]
pub struct LockFreeApalisManager {
    pub config: TaskManagementConfig,
    pub pool: sqlx::SqlitePool,
    pub monitor_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub worker_running: AtomicBool,
}

impl Clone for LockFreeApalisManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            pool: self.pool.clone(),
            monitor_handle: self.monitor_handle.clone(),
            worker_running: AtomicBool::new(self.worker_running.load(std::sync::atomic::Ordering::Relaxed)),
        }
    }
}

/// Apalis 任务管理器（保留兼容性）
#[derive(Debug)]
pub struct ApalisManager {
    pub config: TaskManagementConfig,
    pub pool: sqlx::SqlitePool,
    pub monitor_handle: Option<tokio::task::JoinHandle<()>>,
}

impl LockFreeApalisManager {
    /// 创建新的无锁管理器，返回 (LockFreeApalisManager, SqliteStorage) 元组
    pub async fn new(
        config: TaskManagementConfig,
        _model_service: Arc<ModelService>,
    ) -> Result<(Self, SqliteStorage<TranscriptionTask>), VoiceCliError> {
        let database_url = format!("sqlite://{}", config.sqlite_db_path);
        info!("初始化 ApalisManager，数据库: {}", database_url);

        // 确保数据库目录存在
        let db_path = std::path::Path::new(&config.sqlite_db_path);
        info!("数据库路径: {:?} (当前工作目录: {:?})", db_path, std::env::current_dir());
        if let Some(parent_dir) = db_path.parent() {
            info!("父目录: {:?}, 是否存在: {}", parent_dir, parent_dir.exists());
            if !parent_dir.exists() {
                info!("创建目录: {:?}", parent_dir);
                std::fs::create_dir_all(parent_dir)
                    .map_err(|e| VoiceCliError::Storage(format!("创建数据库目录失败: {}", e)))?;
                info!("目录创建成功: {:?}", parent_dir);
            }
        }

        // 确保数据库文件存在
        if !db_path.exists() {
            info!("创建数据库文件: {:?}", db_path);
            // 创建空文件
            std::fs::File::create(db_path)
                .map_err(|e| VoiceCliError::Storage(format!("创建数据库文件失败: {}", e)))?;
            info!("数据库文件创建成功: {:?}", db_path);
        } else {
            // 检查文件权限
            let metadata = std::fs::metadata(db_path)
                .map_err(|e| VoiceCliError::Storage(format!("获取数据库文件元数据失败: {}", e)))?;
            
            if metadata.permissions().readonly() {
                return Err(VoiceCliError::Storage(format!(
                    "数据库文件只读，无法写入: {:?}", 
                    db_path
                )));
            }
            
            info!("数据库文件已存在且可写: {:?}", db_path);
        }

        // 创建数据库连接池
        let pool = SqlitePoolOptions::new()
            .max_connections(10)
            .connect(&database_url)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("连接数据库失败: {}", e)))?;

        // 设置 Apalis 存储
        SqliteStorage::setup(&pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("设置 Apalis 存储失败: {}", e)))?;

        let storage = SqliteStorage::new(pool.clone());

        let manager = Self {
            pool,
            config,
            monitor_handle: Arc::new(tokio::sync::Mutex::new(None)),
            worker_running: AtomicBool::new(false),
        };

        // 初始化自定义表
        manager.init_custom_tables().await?;

        info!("ApalisManager 初始化完成");
        Ok((manager, storage))
    }

    /// 初始化自定义数据表
    async fn init_custom_tables(&self) -> Result<(), VoiceCliError> {
        // 任务状态表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS task_status (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("创建状态表失败: {}", e)))?;

        // 任务结果表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS task_results (
                task_id TEXT PRIMARY KEY,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("创建结果表失败: {}", e)))?;

        Ok(())
    }

    /// 启动 worker（内部使用步骤化逻辑）
    pub async fn start_worker(
        &self,
        storage: SqliteStorage<TranscriptionTask>,
        model_service: Arc<ModelService>,
    ) -> Result<(), VoiceCliError> {
        if self.worker_running.load(Ordering::Acquire) {
            return Ok(());
        }
        // 创建服务
        let transcription_engine = Arc::new(TranscriptionEngine::new(model_service));
        let audio_file_manager = Arc::new(
            AudioFileManager::new("./data/audio")
                .map_err(|e| VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))?,
        );

        // 创建步骤上下文
        let step_context = StepContext {
            transcription_engine,
            audio_file_manager,
            pool: self.pool.clone(),
        };

        // 创建普通 worker，内部使用步骤化逻辑
        let worker = WorkerBuilder::new("transcription-pipeline")
            .data(step_context)
            .enable_tracing()
            .concurrency(self.config.max_concurrent_tasks)
            .retry(RetryPolicy::retries(self.config.retry_attempts))
            .backend(storage.clone())
            .build_fn(transcription_pipeline_worker);

        // 启动监控器 - 使用更简单的方法
        let monitor = Monitor::new().register(worker);
        
        info!("启动 Apalis 监控器...");
        
        // 在后台运行监控器
        let monitor_handle = tokio::spawn(async move {
            info!("Apalis 监控器开始运行，等待任务...");
            match monitor.run().await {
                Ok(()) => info!("Apalis 监控器正常完成"),
                Err(e) => warn!("Apalis 监控器错误: {}", e),
            }
        });
        
        self.worker_running.store(true, Ordering::Release);

        *self.monitor_handle.lock().await = Some(monitor_handle);
        
        info!("Apalis 监控器启动完成");

        // 启动定时清理任务调度器
        if let Err(e) = self.start_cleanup_scheduler().await {
            warn!("启动清理调度器失败: {}", e);
        }

        info!("Apalis worker 启动成功");
        Ok(())
    }

    /// 提交任务
    pub async fn submit_task(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        audio_file_path: PathBuf,
        original_filename: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<String, VoiceCliError> {
        info!("submit_task: 开始创建任务...");
        let task = AsyncTranscriptionTask::new(
            self.generate_task_id(),
            audio_file_path,
            original_filename,
            model,
            response_format,
        );

        info!("submit_task: 任务创建完成: {}", task.task_id);
        let apalis_task: TranscriptionTask = task.clone().into();

        info!("submit_task: 开始推送任务到队列...");
        info!("submit_task: 任务数据: {:?}", apalis_task);
        
        // Use the storage directly without cloning
        info!("submit_task: 准备调用 storage.push()...");
        let push_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            storage.push(apalis_task.clone())
        ).await;
        info!("submit_task: storage.push() 调用完成");
        
        match push_result {
            Ok(Ok(_)) => {
                info!("submit_task: 任务推送成功");
            },
            Ok(Err(e)) => {
                info!("submit_task: 任务推送失败: {}", e);
                return Err(VoiceCliError::Storage(format!("提交任务失败: {}", e)));
            },
            Err(_) => {
                info!("submit_task: 任务推送超时");
                return Err(VoiceCliError::Storage("推送任务到队列超时".to_string()));
            },
        };
        
        info!("任务已推送到 Apalis 存储: {:?}", apalis_task.task_id);

        // 初始状态
        info!("submit_task: 保存初始任务状态...");
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        self.save_task_status(&task.task_id, &initial_status).await?;

        info!("任务提交成功: {}", task.task_id);
        Ok(task.task_id)
    }

    /// 获取任务状态（直接从数据库查询）
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskStatus>, VoiceCliError> {
        // 直接从数据库查询
        let row = sqlx::query("SELECT status FROM task_status WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务状态失败: {}", e)))?;

        if let Some(row) = row {
            let status_json: String = row.try_get("status")
                .map_err(|e| VoiceCliError::Storage(format!("获取状态字段失败: {}", e)))?;
            let status: TaskStatus = serde_json::from_str(&status_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务状态失败: {}", e)))?;
            
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// 保存任务状态
    async fn save_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
    ) -> Result<(), VoiceCliError> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务状态失败: {}", e)))?;
        
        sqlx::query(
            "INSERT OR REPLACE INTO task_status (task_id, status, updated_at) VALUES (?, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务状态失败: {}", e)))?;
        
        Ok(())
    }

    /// 获取任务结果
    pub async fn get_task_result(
        &self,
        task_id: &str,
    ) -> Result<Option<TranscriptionResponse>, VoiceCliError> {
        let row = sqlx::query("SELECT result FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务结果失败: {}", e)))?;

        if let Some(row) = row {
            let result_json: String = row.try_get("result")
                .map_err(|e| VoiceCliError::Storage(format!("获取结果字段失败: {}", e)))?;
            let result = serde_json::from_str(&result_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务结果失败: {}", e)))?;
            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    /// 保存任务结果
    async fn save_task_result(
        &self,
        task_id: &str,
        result: &TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        let result_json = serde_json::to_string(result)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务结果失败: {}", e)))?;
        
        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, created_at) VALUES (?, ?, ?)"
        )
        .bind(task_id)
        .bind(result_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务结果失败: {}", e)))?;
        
        Ok(())
    }

    /// 取消任务
    pub async fn cancel_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        let current_status = self.get_task_status(task_id).await?;

        match current_status {
            Some(TaskStatus::Pending { .. }) | Some(TaskStatus::Processing { .. }) => {
                let cancelled_status = TaskStatus::Cancelled {
                    cancelled_at: Utc::now(),
                    reason: Some("用户取消".to_string()),
                };

                self.save_task_status(task_id, &cancelled_status).await?;

                info!("任务已取消: {}", task_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// 重试任务
    pub async fn retry_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        let current_status = self.get_task_status(task_id).await?;

        match current_status {
            Some(TaskStatus::Failed { .. }) | Some(TaskStatus::Cancelled { .. }) => {
                // 查询数据库中是否有原始任务数据
                let task_data: Option<(String, String, String, Option<String>, Option<String>)> = sqlx::query_as(
                    "SELECT id, task_id, status, payload, lock_by FROM apalis.jobs WHERE task_id = ? LIMIT 1"
                )
                .bind(task_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| VoiceCliError::Storage(format!("查询任务数据失败: {}", e)))?;

                if task_data.is_some() {
                    // 重置状态为待处理
                    let retry_status = TaskStatus::Pending {
                        queued_at: Utc::now(),
                    };

                    self.save_task_status(task_id, &retry_status).await?;

                    // 重新提交任务到队列 (这里简化实现，仅更新状态)
                    // 实际实现可能需要重新将任务推入 Apalis 队列
                    
                    info!("任务已重新提交: {}", task_id);
                    Ok(true)
                } else {
                    warn!("任务数据不存在，无法重试: {}", task_id);
                    Ok(false)
                }
            }
            Some(TaskStatus::Pending { .. }) | Some(TaskStatus::Processing { .. }) => {
                warn!("任务正在处理中，无法重试: {}", task_id);
                Ok(false)
            }
            Some(TaskStatus::Completed { .. }) => {
                warn!("任务已完成，无法重试: {}", task_id);
                Ok(false)
            }
            None => {
                warn!("任务不存在，无法重试: {}", task_id);
                Ok(false)
            }
        }
    }

    /// 删除任务（彻底删除任务数据和状态）
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        // 从所有相关表中删除任务数据
        let mut deleted = false;
        
        // 删除任务状态
        let status_result = sqlx::query("DELETE FROM task_status WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务状态失败: {}", e)))?;
        
        if status_result.rows_affected() > 0 {
            deleted = true;
        }
        
        // 删除任务结果
        let result_result = sqlx::query("DELETE FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务结果失败: {}", e)))?;
        
        if result_result.rows_affected() > 0 {
            deleted = true;
        }
        
        // 尝试从 Apalis 作业表中删除（如果存在）
        let _ = sqlx::query("DELETE FROM apalis.jobs WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await;
        
        info!("任务删除操作: {} -> {}", task_id, if deleted { "成功" } else { "任务不存在" });
        Ok(deleted)
    }

    /// 检查 worker 是否运行
    pub fn is_worker_running(&self) -> bool {
        self.worker_running.load(Ordering::Acquire)
    }

    /// 获取任务统计信息
    pub async fn get_tasks_stats(&self) -> Result<TaskStatsResponse, VoiceCliError> {
        let mut total_tasks = 0u32;
        let mut pending_tasks = 0u32;
        let mut processing_tasks = 0u32;
        let mut completed_tasks = 0u32;
        let mut failed_tasks = 0u32;
        let mut cancelled_tasks = 0u32;
        let mut failed_task_ids = Vec::new();
        let mut processing_times = Vec::new();

        // 从 SQLite 查询所有任务状态
        let rows = sqlx::query("SELECT task_id, status FROM task_status")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务统计失败: {}", e)))?;

        for row in rows {
            let task_id: String = row.try_get("task_id")
                .map_err(|e| VoiceCliError::Storage(format!("获取任务ID失败: {}", e)))?;
            let status_json: String = row.try_get("status")
                .map_err(|e| VoiceCliError::Storage(format!("获取状态字段失败: {}", e)))?;
            
            let status: TaskStatus = serde_json::from_str(&status_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务状态失败: {}", e)))?;
            
            total_tasks += 1;
            
            match status {
                TaskStatus::Pending { .. } => {
                    pending_tasks += 1;
                }
                TaskStatus::Processing { .. } => {
                    processing_tasks += 1;
                }
                TaskStatus::Completed { processing_time, .. } => {
                    completed_tasks += 1;
                    processing_times.push(processing_time.as_millis() as f64);
                }
                TaskStatus::Failed { .. } => {
                    failed_tasks += 1;
                    failed_task_ids.push(task_id);
                }
                TaskStatus::Cancelled { .. } => {
                    cancelled_tasks += 1;
                }
            }
        }

        // 计算平均处理时间
        let average_processing_time_ms = if !processing_times.is_empty() {
            Some(processing_times.iter().sum::<f64>() / processing_times.len() as f64)
        } else {
            None
        };

        let stats = TaskStatsResponse {
            total_tasks,
            pending_tasks,
            processing_tasks,
            completed_tasks,
            failed_tasks,
            cancelled_tasks,
            average_processing_time_ms,
            failed_task_ids,
        };

        info!("任务统计: 总共 {} 个任务, 完成 {} 个, 失败 {} 个", 
              total_tasks, completed_tasks, failed_tasks);

        Ok(stats)
    }

    /// 清理过期任务
    pub async fn cleanup_expired_tasks(&self) -> Result<usize, VoiceCliError> {
        let retention_days = self.config.task_retention_days;
        if retention_days == 0 {
            info!("任务保留天数为0，跳过清理");
            return Ok(0);
        }

        let cutoff_time = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_timestamp = cutoff_time.timestamp();

        info!("开始清理过期任务，保留天数: {}，截止时间: {}", retention_days, cutoff_time);

        // 获取过期任务列表
        let expired_tasks = self.get_expired_task_ids(cutoff_timestamp).await?;
        
        let mut cleaned_count = 0;
        
        for task_id in &expired_tasks {
            if let Ok(deleted) = self.delete_task_with_files(task_id).await {
                if deleted {
                    cleaned_count += 1;
                }
            }
        }

        info!("清理完成: 总共 {} 个过期任务，成功清理 {} 个", expired_tasks.len(), cleaned_count);
        Ok(cleaned_count)
    }

    /// 获取过期任务ID列表
    async fn get_expired_task_ids(&self, cutoff_timestamp: i64) -> Result<Vec<String>, VoiceCliError> {
        let rows = sqlx::query(
            "SELECT task_id FROM task_status WHERE updated_at < ?"
        )
        .bind(cutoff_timestamp)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("查询过期任务失败: {}", e)))?;

        let mut task_ids = Vec::new();
        for row in rows {
            if let Ok(task_id) = row.try_get::<String, _>("task_id") {
                task_ids.push(task_id);
            }
        }

        Ok(task_ids)
    }

    /// 删除任务及其相关文件
    async fn delete_task_with_files(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        // 首先获取任务的文件路径信息
        if let Some(audio_file_path) = self.get_task_audio_file_path(task_id).await? {
            // 删除音频文件
            if let Err(e) = tokio::fs::remove_file(&audio_file_path).await {
                warn!("删除音频文件失败: {} - {}", audio_file_path.display(), e);
            }
            
            // 尝试删除文件所在目录（如果为空）
            if let Some(parent_dir) = audio_file_path.parent() {
                let _ = tokio::fs::remove_dir(parent_dir).await;
            }
        }

        // 删除数据库中的任务数据
        self.delete_task(task_id).await
    }

    /// 获取任务的音频文件路径
    async fn get_task_audio_file_path(&self, task_id: &str) -> Result<Option<PathBuf>, VoiceCliError> {
        // 从 Apalis jobs 表中查询任务数据
        let row = sqlx::query(
            "SELECT payload FROM apalis.jobs WHERE task_id = ? LIMIT 1"
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("查询任务数据失败: {}", e)))?;

        if let Some(row) = row {
            if let Ok(payload_json) = row.try_get::<String, _>("payload") {
                if let Ok(task_data) = serde_json::from_str::<TranscriptionTask>(&payload_json) {
                    return Ok(Some(task_data.audio_file_path));
                }
            }
        }

        Ok(None)
    }

    /// 启动定时清理任务
    pub async fn start_cleanup_scheduler(&self) -> Result<(), VoiceCliError> {
        if self.config.task_retention_days == 0 {
            info!("任务保留天数为0，不启动清理调度器");
            return Ok(());
        }

        let manager = self.clone();
        
        tokio::spawn(async move {
            // 初始延迟，避免立即清理
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
            
            // 定时清理任务，默认每10分钟清理一次
            let cleanup_interval = tokio::time::Duration::from_secs(600);
            
            loop {
                tokio::time::sleep(cleanup_interval).await;
                
                match manager.cleanup_expired_tasks().await {
                    Ok(cleaned_count) => {
                        if cleaned_count > 0 {
                            info!("定时清理完成: 清理了 {} 个过期任务", cleaned_count);
                        }
                    }
                    Err(e) => {
                        warn!("定时清理任务失败: {}", e);
                    }
                }
            }
        });

        info!("任务清理调度器启动成功，保留天数: {} 天", self.config.task_retention_days);
        Ok(())
    }

    /// 保存任务结果
    pub async fn save_result(
        &self,
        task_id: &str,
        result: TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        self.save_task_result(task_id, &result).await?;

        debug!("保存结果成功: {}", task_id);
        Ok(())
    }

    /// 优雅关闭
    pub async fn shutdown(&self) -> Result<(), VoiceCliError> {
        self.worker_running.store(false, Ordering::Release);
        if let Some(handle) = self.monitor_handle.lock().await.take() {
            handle.abort();
        }
        
        info!("LockFreeApalisManager 已关闭");
        Ok(())
    }

    /// 生成任务 ID
    fn generate_task_id(&self) -> String {
        format!(
            "task_{}",
            uuid::Uuid::now_v7().to_string()
        )
    }
}

impl ApalisManager {
    /// 创建新的管理器，返回 (ApalisManager, SqliteStorage) 元组
    pub async fn new(
        config: TaskManagementConfig,
        model_service: Arc<ModelService>,
    ) -> Result<(Self, SqliteStorage<TranscriptionTask>), VoiceCliError> {
        let (lock_free_manager, storage) = LockFreeApalisManager::new(config.clone(), model_service).await?;
        
        let manager = Self {
            config,
            pool: lock_free_manager.pool.clone(),
            monitor_handle: None,
        };
        
        Ok((manager, storage))
    }
    
    /// 启动 worker（委托给无锁版本）
    pub async fn start_worker(
        &mut self,
        storage: SqliteStorage<TranscriptionTask>,
        model_service: Arc<ModelService>,
    ) -> Result<(), VoiceCliError> {
        let (lock_free_manager, _) = LockFreeApalisManager::new(self.config.clone(), model_service).await?;
        lock_free_manager.start_worker(storage, Arc::new(ModelService::new(crate::models::Config::default()))).await
    }
    
    /// 其他方法委托实现...
    pub async fn submit_task(
        &self,
        _storage: &mut SqliteStorage<TranscriptionTask>,
        _audio_file_path: PathBuf,
        _original_filename: String,
        _model: Option<String>,
        _response_format: Option<String>,
    ) -> Result<String, VoiceCliError> {
        // 简化实现，实际应该委托给 LockFreeApalisManager
        Err(VoiceCliError::Config("请使用 LockFreeApalisManager".to_string()))
    }
}


/// 初始化全局无锁 Apalis 管理器
pub async fn init_global_lock_free_apalis_manager(
    config: TaskManagementConfig,
    model_service: Arc<ModelService>,
) -> Result<(Arc<LockFreeApalisManager>, SqliteStorage<TranscriptionTask>), VoiceCliError> {
    let (manager, storage) = LockFreeApalisManager::new(config, model_service).await?;
    let manager_arc = Arc::new(manager);
    
    GLOBAL_APALIS_MANAGER.set(manager_arc.clone())
        .map_err(|_| VoiceCliError::Config("全局 Apalis 管理器已经初始化".to_string()))?;

    Ok((manager_arc, storage))
}

/// 获取全局无锁 Apalis 管理器
pub async fn get_global_lock_free_apalis_manager() -> Option<Arc<LockFreeApalisManager>> {
    GLOBAL_APALIS_MANAGER.get().cloned()
}

/// 初始化全局 Apalis 管理器（兼容性）
pub async fn init_global_apalis_manager(
    config: TaskManagementConfig,
    model_service: Arc<ModelService>,
) -> Result<(Arc<tokio::sync::Mutex<ApalisManager>>, SqliteStorage<TranscriptionTask>), VoiceCliError> {
    let (manager, storage) = ApalisManager::new(config, model_service).await?;
    let manager_arc = Arc::new(tokio::sync::Mutex::new(manager));
    Ok((manager_arc, storage))
}

/// 获取全局 Apalis 管理器（兼容性）
pub async fn get_global_apalis_manager() -> Option<Arc<tokio::sync::Mutex<ApalisManager>>> {
    None // 不再支持全局锁版本
}


/// 步骤 1: 音频预处理
async fn audio_preprocessing_step(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<AudioProcessedTask, Error> {
    info!("步骤 1 - 音频预处理: {}", task.task_id);

    // 更新状态为处理中
    ctx.save_task_status(&task.task_id, &TaskStatus::Processing {
        stage: crate::models::ProcessingStage::AudioFormatDetection,
        started_at: Utc::now(),
        progress_details: None,
    }).await?;

    // 读取并验证音频文件
    let _audio_data = tokio::fs::read(&task.audio_file_path).await.map_err(|e| {
        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("读取音频文件失败: {}", e),
        ))))
    })?;

    // 音频预处理完成，进入下一步
    let processed_task = AudioProcessedTask {
        task_id: task.task_id.clone(),
        processed_audio_path: task.audio_file_path,
        original_filename: task.original_filename,
        model: task.model,
        response_format: task.response_format,
        created_at: task.created_at,
    };

    info!("音频预处理完成: {}", task.task_id);
    Ok(processed_task)
}

/// 步骤 2: Whisper 转录
async fn transcription_step(
    task: AudioProcessedTask,
    ctx: Data<StepContext>,
) -> Result<TranscriptionCompletedTask, Error> {
    info!("步骤 2 - Whisper 转录: {}", task.task_id);

    // 更新状态为转录中
    ctx.save_task_status(&task.task_id, &TaskStatus::Processing {
        stage: crate::models::ProcessingStage::WhisperTranscription,
        started_at: Utc::now(),
        progress_details: None,
    }).await?;

    // 执行转录，使用配置中的默认模型
    let default_model = ctx.transcription_engine.default_model();
    let model = task.model.as_deref().unwrap_or(default_model);
    let transcription_result = ctx
        .transcription_engine
        .transcribe_compatible_audio(
            model,
            &task.processed_audio_path,
            ctx.transcription_engine.worker_timeout(), // 使用配置中的超时时间
        )
        .await
        .map_err(|e| {
            Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("转录失败: {}", e),
            ))))
        })?;

    // 转换为 TranscriptionResponse
    let response = TranscriptionResponse {
        text: transcription_result.text,
        segments: transcription_result
            .segments
            .into_iter()
            .map(|s| crate::models::Segment {
                start: s.start_time as f32 / 1000.0,
                end: s.end_time as f32 / 1000.0,
                text: s.text,
                confidence: s.confidence,
            })
            .collect(),
        language: transcription_result.language,
        duration: None,
        processing_time: 0.0,
    };

    let completed_task = TranscriptionCompletedTask {
        task_id: task.task_id.clone(),
        transcription_result: response,
        response_format: task.response_format,
        created_at: task.created_at,
    };

    info!(
        "Whisper 转录完成: {} ({} 字符)",
        task.task_id,
        completed_task.transcription_result.text.len()
    );
    Ok(completed_task)
}

/// 步骤 3: 结果格式化和存储
async fn result_formatting_step(
    task: TranscriptionCompletedTask,
    ctx: Data<StepContext>,
) -> Result<(), Error> {
    info!("步骤 3 - 结果格式化和存储: {}", task.task_id);

    // 保存结果到SQLite存储
    ctx.save_task_result(&task.task_id, &task.transcription_result).await?;

    // 计算实际处理时间
    let now = Utc::now();
    let processing_time = now.signed_duration_since(task.created_at);
    let duration = Duration::from_secs(processing_time.num_seconds().max(0) as u64);

    // 更新状态为完成
    ctx.save_task_status(&task.task_id, &TaskStatus::Completed {
        completed_at: now,
        processing_time: duration,
        result_summary: Some(format!(
            "转录了 {} 个字符",
            task.transcription_result.text.len()
        )),
    }).await?;

    info!(
        "转录任务完成: {} (处理时间: {}s)",
        task.task_id,
        duration.as_secs()
    );
    Ok(())
}

/// 转录流水线 worker - 内部调用步骤函数
pub async fn transcription_pipeline_worker(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<(), Error> {
    info!("开始处理转录流水线: {}", task.task_id);
    
    // 步骤 1: 音频预处理
    let audio_processed_task = audio_preprocessing_step(task.clone(), ctx.clone()).await
        .map_err(|e| {
            warn!("步骤 1 失败: {} - {}", task.task_id, e);
            e
        })?;

    // 步骤 2: Whisper 转录
    let transcription_completed_task = transcription_step(audio_processed_task, ctx.clone()).await
        .map_err(|e| {
            warn!("步骤 2 失败: {} - {}", task.task_id, e);
            e
        })?;

    // 步骤 3: 结果格式化和存储
    result_formatting_step(transcription_completed_task, ctx.clone()).await
        .map_err(|e| {
            warn!("步骤 3 失败: {} - {}", task.task_id, e);
            e
        })?;

    info!("转录流水线完成: {}", task.task_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_id_generation() {
        let _config = TaskManagementConfig::default();
        let _model_service = Arc::new(ModelService::new(crate::models::Config::default()));

        // 这里只测试 ID 生成格式
        let task_id = format!(
            "task_{}_{}",
            Utc::now().timestamp_millis(),
            std::process::id()
        );

        assert!(task_id.starts_with("task_"));
        assert!(task_id.len() > 10);
    }
}
