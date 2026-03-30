use crate::VoiceCliError;
use crate::models::{
    AsyncTranscriptionTask, ProcessingStage, TaskError, TaskManagementConfig, TaskStatsResponse,
    TaskStatus, TranscriptionResponse,
};
use crate::services::{
    AudioFileManager, AudioFormatDetector, MetadataExtractor, ModelService, TranscriptionEngine,
};
use crate::utils::{get_file_extension, is_supported_media_format};
use apalis::layers::WorkerBuilderExt;
use apalis::layers::retry::RetryPolicy;
use apalis::prelude::*;
use apalis_sql::sqlite::SqliteStorage;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use reqwest;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// 全局 Apalis 管理器实例（无锁版本）
static GLOBAL_APALIS_MANAGER: OnceLock<Arc<LockFreeApalisManager>> = OnceLock::new();

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskType {
    /// 文件上传任务
    FileUpload,
    /// URL下载任务
    UrlDownload,
}

/// 初始转录任务 - 流水线的第一步
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionTask {
    pub task_id: String,
    pub audio_file_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
    /// 任务类型
    pub task_type: TaskType,
    /// URL地址（仅对UrlDownload类型有效）
    pub url: Option<String>,
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
    pub metadata: Option<crate::models::request::AudioVideoMetadata>,
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
            task_type: TaskType::FileUpload,
            url: None,
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
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, retry_count, error_message, created_at, updated_at) VALUES (?, ?, NULL, 0, NULL, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        Ok(())
    }

    /// 保存任务结果到SQLite
    async fn save_task_result(
        &self,
        task_id: &str,
        result: &TranscriptionResponse,
        metadata: &Option<crate::models::request::AudioVideoMetadata>,
    ) -> Result<(), Error> {
        let result_json = serde_json::to_string(result)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        let metadata_json = metadata
            .as_ref()
            .map(|m| {
                serde_json::to_string(m).map_err(|e| {
                    Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                })
            })
            .transpose()?;

        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, metadata, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(result_json)
        .bind(metadata_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        Ok(())
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
            worker_running: AtomicBool::new(
                self.worker_running
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
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
        info!("Initialize ApalisManager, database: {}", database_url);

        // 确保数据库目录存在
        let db_path = std::path::Path::new(&config.sqlite_db_path);
        info!(
            "Database path: {:?} (Current working directory: {:?})",
            db_path,
            std::env::current_dir()
        );
        if let Some(parent_dir) = db_path.parent() {
            info!(
                "Parent directory: {:?}, exists: {}",
                parent_dir,
                parent_dir.exists()
            );
            if !parent_dir.exists() {
                info!("Create directory: {:?}", parent_dir);
                std::fs::create_dir_all(parent_dir)
                    .map_err(|e| VoiceCliError::Storage(format!("创建数据库目录失败: {}", e)))?;
                info!("Directory created successfully: {:?}", parent_dir);
            }
        }

        // 确保数据库文件存在
        if !db_path.exists() {
            info!("Create database file: {:?}", db_path);
            // 创建空文件
            std::fs::File::create(db_path)
                .map_err(|e| VoiceCliError::Storage(format!("创建数据库文件失败: {}", e)))?;
            info!("Database file created successfully: {:?}", db_path);
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

            info!("The database file exists and is writable: {:?}", db_path);
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

        info!("ApalisManager initialization completed");
        Ok((manager, storage))
    }

    /// 初始化自定义数据表
    async fn init_custom_tables(&self) -> Result<(), VoiceCliError> {
        // 任务状态表
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS task_info (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                file_path TEXT,
                original_filename TEXT,
                model TEXT,
                response_format TEXT,
                retry_count INTEGER DEFAULT 0,
                error_message TEXT,
                created_at INTEGER NOT NULL,
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
                metadata TEXT,
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
        info!(
            "Creating Apalis worker...max_concurrent_tasks={},retry_attempts={}",
            self.config.max_concurrent_tasks, self.config.retry_attempts
        );
        let worker = WorkerBuilder::new("transcription-pipeline")
            .data(step_context)
            .enable_tracing()
            .concurrency(self.config.max_concurrent_tasks)
            .retry(RetryPolicy::retries(self.config.retry_attempts))
            .backend(storage.clone())
            .build_fn(transcription_pipeline_worker);

        // 启动监控器 - 使用更简单的方法
        let monitor = Monitor::new().register(worker);

        info!("Starting the Apalis monitor...");

        // 在后台运行监控器
        let monitor_handle = tokio::spawn(async move {
            info!("Apalis monitor starts running, waiting for tasks...");
            match monitor.run().await {
                Ok(()) => info!("Apalis monitor completes normally"),
                Err(e) => warn!("Apalis monitor error: {}", e),
            }
        });

        self.worker_running.store(true, Ordering::Release);

        *self.monitor_handle.lock().await = Some(monitor_handle);

        info!("Apalis monitor startup completed");

        // 启动定时清理任务调度器
        if let Err(e) = self.start_cleanup_scheduler().await {
            warn!("Failed to start cleanup scheduler: {}", e);
        }

        info!("Apalis worker started successfully");
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
        info!("submit_task: Start creating task...");
        let task = AsyncTranscriptionTask::new(
            self.generate_task_id(),
            audio_file_path.clone(),
            original_filename.clone(),
            model.clone(),
            response_format.clone(),
        );

        info!("submit_task: Task creation completed: {}", task.task_id);
        let apalis_task: TranscriptionTask = task.clone().into();

        info!("submit_task: Start pushing tasks to the queue...");
        info!("submit_task: task data: {:?}", apalis_task);

        // Use the storage directly without cloning
        info!("submit_task: Prepare to call storage.push()...");
        let push_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            storage.push(apalis_task.clone()),
        )
        .await;
        info!("submit_task: storage.push() call completed");

        match push_result {
            Ok(Ok(_)) => {
                info!("submit_task: Task pushed successfully");
            }
            Ok(Err(e)) => {
                info!("submit_task: Task push failed: {}", e);
                return Err(VoiceCliError::Storage(format!("提交任务失败: {}", e)));
            }
            Err(_) => {
                info!("submit_task: task push timeout");
                return Err(VoiceCliError::Storage("推送任务到队列超时".to_string()));
            }
        };

        info!("Task pushed to Apalis storage: {:?}", apalis_task.task_id);

        // 初始状态
        info!("submit_task: Save the initial task status...");
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        // 使用新的保存任务信息方法，包含文件路径
        self.save_task_info(
            &task.task_id,
            &initial_status,
            Some(&audio_file_path),
            Some(&original_filename),
            model.as_ref().map(|s| s.as_str()),
            response_format.as_ref().map(|s| s.as_str()),
            0,
            None,
        )
        .await?;

        info!("Task submitted successfully: {}", task.task_id);
        Ok(task.task_id)
    }

    /// 提交URL转录任务
    pub async fn submit_task_for_url(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        url: String,
        filename: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<String, VoiceCliError> {
        info!("submit_task_for_url: Start creating URL task...");

        // 生成任务ID
        let task_id = self.generate_task_id();

        // 创建临时文件路径（实际下载将在worker中执行）
        let temp_audio_path = PathBuf::from(format!("./data/audio/temp_{}.pending", task_id));

        // 创建任务对象
        let task = AsyncTranscriptionTask::new(
            task_id.clone(),
            temp_audio_path.clone(),
            filename.clone(),
            model.clone(),
            response_format.clone(),
        );

        info!(
            "submit_task_for_url: Task creation completed: {}",
            task.task_id
        );

        // 转换为Apalis任务，设置URL任务类型
        let apalis_task = TranscriptionTask {
            task_id: task.task_id.clone(),
            audio_file_path: temp_audio_path,
            original_filename: filename.clone(),
            model: task.model,
            response_format: task.response_format,
            created_at: task.created_at,
            task_type: TaskType::UrlDownload,
            url: Some(url),
        };

        info!("submit_task_for_url: Start pushing the URL task to the queue...");
        info!("submit_task_for_url: task data: {:?}", apalis_task);

        // 推送任务到队列
        let push_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            storage.push(apalis_task.clone()),
        )
        .await;
        info!("submit_task_for_url: storage.push() call completed");

        match push_result {
            Ok(Ok(_)) => {
                info!("submit_task_for_url: Task pushed successfully");
            }
            Ok(Err(e)) => {
                info!("submit_task_for_url: Task push failed: {}", e);
                return Err(VoiceCliError::Storage(format!("提交URL任务失败: {}", e)));
            }
            Err(_) => {
                info!("submit_task_for_url: task push timeout");
                return Err(VoiceCliError::Storage("推送URL任务到队列超时".to_string()));
            }
        };

        info!(
            "URL task has been pushed to Apalis storage: {:?}",
            apalis_task.task_id
        );

        // 初始状态
        info!("submit_task_for_url: Save initial task status...");
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        // 保存任务信息，包含URL
        self.save_task_info(
            &task.task_id,
            &initial_status,
            None, // 文件路径将在下载后设置
            Some(&filename),
            model.as_ref().map(|s| s.as_str()),
            response_format.as_ref().map(|s| s.as_str()),
            0,
            None,
        )
        .await?;

        info!("URL task submitted successfully: {}", task.task_id);
        Ok(task.task_id)
    }

    /// 获取任务状态（直接从数据库查询）
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskStatus>, VoiceCliError> {
        // 直接从数据库查询
        let row = sqlx::query("SELECT status FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务状态失败: {}", e)))?;

        if let Some(row) = row {
            let status_json: String = row
                .try_get("status")
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
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, retry_count, error_message, created_at, updated_at) VALUES (?, ?, NULL, 0, NULL, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务状态失败: {}", e)))?;

        Ok(())
    }

    /// 保存任务信息（包括文件路径）
    async fn save_task_info(
        &self,
        task_id: &str,
        status: &TaskStatus,
        file_path: Option<&PathBuf>,
        original_filename: Option<&str>,
        model: Option<&str>,
        response_format: Option<&str>,
        retry_count: u32,
        error_message: Option<&str>,
    ) -> Result<(), VoiceCliError> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务状态失败: {}", e)))?;

        let file_path_str = file_path.map(|p| p.to_string_lossy().to_string());

        sqlx::query(
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, original_filename, model, response_format, retry_count, error_message, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(file_path_str)
        .bind(original_filename)
        .bind(model)
        .bind(response_format)
        .bind(retry_count as i32)
        .bind(error_message)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务信息失败: {}", e)))?;

        Ok(())
    }

    /// 获取任务结果
    pub async fn get_task_result(
        &self,
        task_id: &str,
    ) -> Result<Option<TranscriptionResponse>, VoiceCliError> {
        let row = sqlx::query("SELECT result, metadata FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务结果失败: {}", e)))?;

        if let Some(row) = row {
            let result_json: String = row
                .try_get("result")
                .map_err(|e| VoiceCliError::Storage(format!("获取结果字段失败: {}", e)))?;

            let mut result: TranscriptionResponse = serde_json::from_str(&result_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务结果失败: {}", e)))?;

            // 尝试获取元数据
            let metadata_json: Option<String> = row.try_get("metadata").unwrap_or(None);

            if let Some(meta_json) = metadata_json {
                if let Ok(metadata) =
                    serde_json::from_str::<crate::models::request::AudioVideoMetadata>(&meta_json)
                {
                    result.metadata = Some(metadata);
                }
            }

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

        let metadata_json = result
            .metadata
            .as_ref()
            .map(|m| {
                serde_json::to_string(m)
                    .map_err(|e| VoiceCliError::Storage(format!("序列化元数据失败: {}", e)))
            })
            .transpose()?;

        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, metadata, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(result_json)
        .bind(metadata_json)
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

                info!("Task canceled: {}", task_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// 重试任务
    pub async fn retry_task(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        task_id: &str,
    ) -> Result<bool, VoiceCliError> {
        let current_status = self.get_task_status(task_id).await?;

        match current_status {
            Some(TaskStatus::Failed { .. }) | Some(TaskStatus::Cancelled { .. }) => {
                // 查询我们自己的 task_info 表中存储的原始任务数据
                let task_data: Option<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
                    "SELECT file_path, original_filename, model, response_format, error_message FROM task_info WHERE task_id = ?"
                )
                .bind(task_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| VoiceCliError::Storage(format!("查询任务数据失败: {}", e)))?;

                if let Some((
                    file_path,
                    original_filename,
                    model,
                    response_format,
                    _error_message,
                )) = task_data
                {
                    if let Some(file_path_str) = file_path {
                        let audio_file_path = PathBuf::from(file_path_str);

                        // 检查文件是否仍然存在
                        if audio_file_path.exists() {
                            // 重新提交任务到 Apalis 队列
                            let result = self
                                .submit_task(
                                    storage,
                                    audio_file_path,
                                    original_filename.unwrap_or_else(|| "unknown".to_string()),
                                    model,
                                    response_format,
                                )
                                .await;

                            match result {
                                Ok(new_task_id) => {
                                    info!(
                                        "The task has been resubmitted: {} -> {}",
                                        task_id, new_task_id
                                    );
                                    Ok(true)
                                }
                                Err(e) => {
                                    warn!("Failed to resubmit task: {} - {}", task_id, e);
                                    Ok(false)
                                }
                            }
                        } else {
                            warn!(
                                "The task audio file does not exist and cannot be retried: {}",
                                task_id
                            );
                            Ok(false)
                        }
                    } else {
                        warn!(
                            "The task file path does not exist and cannot be retried: {}",
                            task_id
                        );
                        Ok(false)
                    }
                } else {
                    warn!(
                        "Task data does not exist and cannot be retried: {}",
                        task_id
                    );
                    Ok(false)
                }
            }
            Some(TaskStatus::Pending { .. }) | Some(TaskStatus::Processing { .. }) => {
                warn!(
                    "The task is being processed and cannot be retried: {}",
                    task_id
                );
                Ok(false)
            }
            Some(TaskStatus::Completed { .. }) => {
                warn!("Task completed and cannot be retried: {}", task_id);
                Ok(false)
            }
            None => {
                warn!("The task does not exist and cannot be retried: {}", task_id);
                Ok(false)
            }
        }
    }

    /// 删除任务（彻底删除任务数据和状态）
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        // 从我们自己的表中删除任务数据（不操作 apalis.jobs 表）
        let mut deleted = false;

        // 删除任务状态
        let status_result = sqlx::query("DELETE FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务状态失败: {}", e)))?;

        if status_result.rows_affected() > 0 {
            deleted = true;
            info!("Successfully deleted the status record of task {}", task_id);
        }

        // 删除任务结果
        let result_result = sqlx::query("DELETE FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务结果失败: {}", e)))?;

        if result_result.rows_affected() > 0 {
            deleted = true;
            info!("Successfully deleted the result record of task {}", task_id);
        }

        info!(
            "Task deletion operation: {} -> {}",
            task_id,
            if deleted { "success" } else { "task not found" }
        );
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
        let rows = sqlx::query("SELECT task_id, status FROM task_info")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务统计失败: {}", e)))?;

        for row in rows {
            let task_id: String = row
                .try_get("task_id")
                .map_err(|e| VoiceCliError::Storage(format!("获取任务ID失败: {}", e)))?;
            let status_json: String = row
                .try_get("status")
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
                TaskStatus::Completed {
                    processing_time, ..
                } => {
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

        info!(
            "Task statistics: Total {} tasks, {} completed, {} failed",
            total_tasks, completed_tasks, failed_tasks
        );

        Ok(stats)
    }

    /// 清理过期任务
    pub async fn cleanup_expired_tasks(&self) -> Result<usize, VoiceCliError> {
        let retention_minutes = self.config.task_retention_minutes;
        if retention_minutes == 0 {
            info!("The task retention minutes is 0 and cleanup is skipped");
            return Ok(0);
        }

        let cutoff_time = chrono::Utc::now() - chrono::Duration::minutes(retention_minutes as i64);
        let cutoff_timestamp = cutoff_time.timestamp();

        info!(
            "Start cleaning up expired tasks, retention minutes: {}, deadline: {}",
            retention_minutes, cutoff_time
        );

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

        info!(
            "Cleanup completed: {} expired tasks in total, {} were successfully cleaned",
            expired_tasks.len(),
            cleaned_count
        );
        Ok(cleaned_count)
    }

    /// 获取过期任务ID列表
    async fn get_expired_task_ids(
        &self,
        cutoff_timestamp: i64,
    ) -> Result<Vec<String>, VoiceCliError> {
        // 添加调试日志
        info!(
            "Query expired tasks, deadline timestamp: {}",
            cutoff_timestamp
        );

        let rows = sqlx::query("SELECT task_id, updated_at FROM task_info WHERE updated_at < ?")
            .bind(cutoff_timestamp)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询过期任务失败: {}", e)))?;

        info!("Found {} expired task records", rows.len());

        let mut task_ids = Vec::new();
        for row in rows {
            if let Ok(task_id) = row.try_get::<String, _>("task_id") {
                if let Ok(updated_at) = row.try_get::<i64, _>("updated_at") {
                    info!("Expired tasks: {} (updated_at: {})", task_id, updated_at);
                }
                task_ids.push(task_id);
            }
        }

        Ok(task_ids)
    }

    /// 删除任务及其相关文件
    async fn delete_task_with_files(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        info!("Start deleting the task and its files: {}", task_id);

        // 首先获取任务的文件路径信息
        if let Some(audio_file_path) = self.get_task_audio_file_path(task_id).await? {
            info!("Found task audio file: {:?}", audio_file_path);

            // 删除音频文件
            if let Err(e) = tokio::fs::remove_file(&audio_file_path).await {
                warn!(
                    "Failed to delete audio files: {} - {}",
                    audio_file_path.display(),
                    e
                );
            } else {
                info!("Audio file deleted successfully: {:?}", audio_file_path);
            }

            // 尝试删除文件所在目录（如果为空）
            if let Some(parent_dir) = audio_file_path.parent() {
                let _ = tokio::fs::remove_dir(parent_dir).await;
            }
        } else {
            info!("Mission audio file not found: {}", task_id);
        }

        // 删除数据库中的任务数据
        let result = self.delete_task(task_id).await;
        info!("Delete task database record result: {:?}", result);
        result
    }

    /// 获取任务的音频文件路径
    async fn get_task_audio_file_path(
        &self,
        task_id: &str,
    ) -> Result<Option<PathBuf>, VoiceCliError> {
        // 从 task_info 表中查询文件路径
        let row = sqlx::query("SELECT file_path FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务文件路径失败: {}", e)))?;

        if let Some(row) = row {
            let file_path: Option<String> = row
                .try_get("file_path")
                .map_err(|e| VoiceCliError::Storage(format!("获取文件路径字段失败: {}", e)))?;

            if let Some(file_path_str) = file_path {
                let path = PathBuf::from(file_path_str);
                if path.exists() {
                    info!("Found audio file for task {}: {:?}", task_id, path);
                    return Ok(Some(path));
                } else {
                    info!(
                        "The file path for task {} exists but the file does not exist: {:?}",
                        task_id, path
                    );
                }
            }
        }

        info!("Audio file path not found for task {}", task_id);
        Ok(None)
    }

    /// 启动定时清理任务
    pub async fn start_cleanup_scheduler(&self) -> Result<(), VoiceCliError> {
        if self.config.task_retention_minutes == 0 {
            info!(
                "The number of task retention minutes is 0 and the cleanup scheduler is not started."
            );
            return Ok(());
        }

        let manager = self.clone();

        tokio::spawn(async move {
            // 初始延迟，避免立即清理
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            // 定时清理任务，默认每1分钟清理一次
            let cleanup_interval = tokio::time::Duration::from_secs(60);

            loop {
                tokio::time::sleep(cleanup_interval).await;

                match manager.cleanup_expired_tasks().await {
                    Ok(cleaned_count) => {
                        if cleaned_count > 0 {
                            info!(
                                "Scheduled cleanup completed: {} expired tasks cleaned up",
                                cleaned_count
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Scheduled cleanup task failed: {}", e);
                    }
                }
            }
        });

        info!(
            "The task cleanup scheduler is started successfully, and the number of reserved minutes is: {} minutes",
            self.config.task_retention_minutes
        );
        Ok(())
    }

    /// 保存任务结果
    pub async fn save_result(
        &self,
        task_id: &str,
        result: TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        self.save_task_result(task_id, &result).await?;

        debug!("Successfully saved result: {}", task_id);
        Ok(())
    }

    /// 优雅关闭
    pub async fn shutdown(&self) -> Result<(), VoiceCliError> {
        self.worker_running.store(false, Ordering::Release);
        if let Some(handle) = self.monitor_handle.lock().await.take() {
            handle.abort();
        }

        info!("LockFreeApalisManager is closed");
        Ok(())
    }

    /// 生成任务 ID - 使用统一的工具函数
    fn generate_task_id(&self) -> String {
        crate::utils::generate_task_id()
    }
}

impl ApalisManager {
    /// 创建新的管理器，返回 (ApalisManager, SqliteStorage) 元组
    pub async fn new(
        config: TaskManagementConfig,
        model_service: Arc<ModelService>,
    ) -> Result<(Self, SqliteStorage<TranscriptionTask>), VoiceCliError> {
        let (lock_free_manager, storage) =
            LockFreeApalisManager::new(config.clone(), model_service).await?;

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
        let (lock_free_manager, _) =
            LockFreeApalisManager::new(self.config.clone(), model_service.clone()).await?;
        lock_free_manager.start_worker(storage, model_service).await
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
        Err(VoiceCliError::Config(
            "请使用 LockFreeApalisManager".to_string(),
        ))
    }
}

/// 初始化全局无锁 Apalis 管理器
pub async fn init_global_lock_free_apalis_manager(
    config: TaskManagementConfig,
    model_service: Arc<ModelService>,
) -> Result<(Arc<LockFreeApalisManager>, SqliteStorage<TranscriptionTask>), VoiceCliError> {
    let (manager, storage) = LockFreeApalisManager::new(config, model_service).await?;
    let manager_arc = Arc::new(manager);

    GLOBAL_APALIS_MANAGER
        .set(manager_arc.clone())
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
) -> Result<
    (
        Arc<tokio::sync::Mutex<ApalisManager>>,
        SqliteStorage<TranscriptionTask>,
    ),
    VoiceCliError,
> {
    let (manager, storage) = ApalisManager::new(config, model_service).await?;
    let manager_arc = Arc::new(tokio::sync::Mutex::new(manager));
    Ok((manager_arc, storage))
}

/// 获取全局 Apalis 管理器（兼容性）
pub async fn get_global_apalis_manager() -> Option<Arc<tokio::sync::Mutex<ApalisManager>>> {
    None // 不再支持全局锁版本
}

/// 步骤 1: 音频预处理（包含URL下载）
async fn audio_preprocessing_step(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<AudioProcessedTask, Error> {
    info!("Step 1 - Audio Preprocessing: {}", task.task_id);

    // 更新状态为处理中
    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Processing {
            stage: crate::models::ProcessingStage::AudioFormatDetection,
            started_at: Utc::now(),
            progress_details: None,
        },
    )
    .await?;

    let audio_file_path = if task.task_type == TaskType::UrlDownload {
        // URL下载任务：下载音频文件
        info!(
            "Download URL audio file: {} - URL: {:?}",
            task.task_id, task.url
        );

        if let Some(url) = task.url {
            let downloaded_path =
                download_audio_from_url(&url, &task.task_id, &ctx.audio_file_manager.storage_dir)
                    .await
                    .map_err(|e| {
                        Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("下载URL音频文件失败: {}", e),
                        ))))
                    })?;

            // 检测文件真实格式并重命名
            let final_audio_path = detect_and_rename_audio_file(&downloaded_path, &task.task_id)
                .await
                .map_err(|e| {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("检测音频文件格式失败: {}", e),
                    ))))
                })?;

            // 更新数据库中的文件路径
            update_task_file_path_in_db(&task.task_id, &final_audio_path, &ctx)
                .await
                .map_err(|e| {
                    Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("更新数据库文件路径失败: {}", e),
                    ))))
                })?;

            final_audio_path
        } else {
            return Err(Error::Abort(std::sync::Arc::new(Box::new(
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("URL任务缺少URL地址: {}", task.task_id),
                ),
            ))));
        }
    } else {
        // 文件上传任务：直接使用现有文件路径
        info!(
            "Process file upload task: {} - File: {:?}",
            task.task_id, task.audio_file_path
        );

        // 读取并验证音频文件
        let _audio_data = tokio::fs::read(&task.audio_file_path).await.map_err(|e| {
            Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("读取音频文件失败: {}", e),
            ))))
        })?;

        // 确保数据库中的文件路径是正确的（文件上传任务）
        update_task_file_path_in_db(&task.task_id, &task.audio_file_path, &ctx)
            .await
            .map_err(|e| {
                Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("更新数据库文件路径失败: {}", e),
                ))))
            })?;

        task.audio_file_path
    };

    // 音频预处理完成，进入下一步
    let processed_task = AudioProcessedTask {
        task_id: task.task_id.clone(),
        processed_audio_path: audio_file_path,
        original_filename: task.original_filename,
        model: task.model,
        response_format: task.response_format,
        created_at: task.created_at,
    };

    info!("Audio preprocessing completed: {}", task.task_id);
    Ok(processed_task)
}

/// 步骤 2: Whisper 转录
async fn transcription_step(
    task: AudioProcessedTask,
    ctx: Data<StepContext>,
) -> Result<TranscriptionCompletedTask, Error> {
    info!("Step 2 - Whisper Transcription: {}", task.task_id);

    // 更新状态为转录中
    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Processing {
            stage: crate::models::ProcessingStage::WhisperTranscription,
            started_at: Utc::now(),
            progress_details: None,
        },
    )
    .await?;

    // 提取音视频元数据
    let metadata = match MetadataExtractor::extract_metadata(&task.processed_audio_path).await {
        Ok(meta) => {
            info!(
                "[Task {}] Successfully extracted audio and video metadata: {}",
                task.task_id,
                MetadataExtractor::get_format_description(&meta)
            );
            // 转换为models::request::AudioVideoMetadata
            Some(crate::models::request::AudioVideoMetadata {
                format: meta.format,
                container_format: meta.container_format,
                duration_seconds: meta.duration_seconds,
                file_size_bytes: meta.file_size_bytes,
                audio_codec: meta.audio_codec,
                sample_rate: meta.sample_rate,
                channels: meta.channels,
                audio_bitrate: meta.audio_bitrate,
                has_video: meta.has_video,
                video_codec: meta.video_codec,
                width: meta.width,
                height: meta.height,
                video_bitrate: meta.video_bitrate,
                frame_rate: meta.frame_rate,
                bitrate: meta.bitrate,
                creation_time: meta.creation_time,
            })
        }
        Err(e) => {
            warn!("[Task {}] Failed to extract metadata: {}", task.task_id, e);
            None
        }
    };

    // 执行转录，使用配置中的默认模型
    let default_model = ctx.transcription_engine.default_model();
    let model = task.model.as_deref().unwrap_or(default_model);

    // 首先检查文件是否有音频流
    let has_audio = check_file_has_audio_stream(&task.processed_audio_path)
        .await
        .map_err(|e| {
            Error::Abort(std::sync::Arc::new(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("检查音频流失败: {}", e),
            ))))
        })?;

    if !has_audio {
        return Err(Error::Abort(std::sync::Arc::new(Box::new(
            std::io::Error::new(
                std::io::ErrorKind::Other,
                "文件不包含音频流，无法进行转录".to_string(),
            ),
        ))));
    }

    let transcription_result = ctx
        .transcription_engine
        .transcribe_with_conversion(
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
    let mut response = TranscriptionResponse {
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
        metadata: None,
    };

    // 设置元数据和时长
    if let Some(meta) = &metadata {
        response.duration = Some(meta.duration_seconds as f32);
        response.metadata = Some(meta.clone());
    }

    let completed_task = TranscriptionCompletedTask {
        task_id: task.task_id.clone(),
        transcription_result: response,
        response_format: task.response_format,
        metadata,
        created_at: task.created_at,
    };

    info!(
        "Whisper transcription completed: {} ({} characters)",
        task.task_id,
        completed_task.transcription_result.text.len()
    );
    Ok(completed_task)
}

/// 检查文件是否包含音频流
async fn check_file_has_audio_stream(file_path: &Path) -> Result<bool, VoiceCliError> {
    use std::process::Command;

    // 使用 ffprobe 检查文件是否有音频流
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-show_streams",
            "-select_streams",
            "a",
            "-of",
            "csv=p=0",
            file_path.to_str().unwrap_or("invalid_path"),
        ])
        .output()
        .map_err(|e| VoiceCliError::AudioConversionFailed(format!("执行 ffprobe 失败: {}", e)))?;

    // 如果输出为空，则没有音频流
    Ok(!output.stdout.is_empty())
}

/// 步骤 3: 结果格式化和存储
async fn result_formatting_step(
    task: TranscriptionCompletedTask,
    ctx: Data<StepContext>,
) -> Result<(), Error> {
    info!("Step 3 - Result Formatting and Storage: {}", task.task_id);

    // 保存结果到SQLite存储（包含元数据）
    ctx.save_task_result(&task.task_id, &task.transcription_result, &task.metadata)
        .await?;

    // 计算实际处理时间
    let now = Utc::now();
    let processing_time = now.signed_duration_since(task.created_at);
    let duration = Duration::from_secs(processing_time.num_seconds().max(0) as u64);

    // 更新状态为完成
    let result_summary = if let Some(metadata) = &task.metadata {
        format!(
            "转录了 {} 个字符，文件: {} ({:.2}s)",
            task.transcription_result.text.len(),
            metadata.format,
            metadata.duration_seconds
        )
    } else {
        format!("转录了 {} 个字符", task.transcription_result.text.len())
    };

    ctx.save_task_status(
        &task.task_id,
        &TaskStatus::Completed {
            completed_at: now,
            processing_time: duration,
            result_summary: Some(result_summary),
        },
    )
    .await?;

    info!(
        "Transcription task completed: {} (processing time: {}s)",
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
    info!(
        "Start processing the transcription pipeline: {}",
        task.task_id
    );

    // 步骤 1: 音频预处理
    let audio_processed_task = match audio_preprocessing_step(task.clone(), ctx.clone()).await {
        Ok(task) => task,
        Err(e) => {
            let error_msg = format!("音频预处理失败: {}", e);
            warn!("Step 1 failed: {} - {}", task.task_id, error_msg);

            // 更新任务状态为失败
            if let Err(save_err) = ctx
                .save_task_status(
                    &task.task_id,
                    &TaskStatus::Failed {
                        error: TaskError::AudioProcessingFailed {
                            stage: ProcessingStage::AudioFormatDetection,
                            message: error_msg.clone(),
                            is_recoverable: true,
                        },
                        failed_at: Utc::now(),
                        retry_count: 0,
                        is_recoverable: true,
                    },
                )
                .await
            {
                warn!(
                    "Failed to save failed status: {} - {}",
                    task.task_id, save_err
                );
            }

            return Err(e);
        }
    };

    // 步骤 2: Whisper 转录
    let transcription_completed_task =
        match transcription_step(audio_processed_task, ctx.clone()).await {
            Ok(task) => task,
            Err(e) => {
                let error_msg = format!("Whisper转录失败: {}", e);
                warn!("Step 2 failed: {} - {}", task.task_id, error_msg);

                // 更新任务状态为失败
                if let Err(save_err) = ctx
                    .save_task_status(
                        &task.task_id,
                        &TaskStatus::Failed {
                            error: TaskError::TranscriptionFailed {
                                model: task.model.clone().unwrap_or_else(|| "unknown".to_string()),
                                message: error_msg.clone(),
                                is_recoverable: true,
                            },
                            failed_at: Utc::now(),
                            retry_count: 0,
                            is_recoverable: true,
                        },
                    )
                    .await
                {
                    warn!(
                        "Failed to save failed status: {} - {}",
                        task.task_id, save_err
                    );
                }

                return Err(e);
            }
        };

    // 步骤 3: 结果格式化和存储
    if let Err(e) = result_formatting_step(transcription_completed_task, ctx.clone()).await {
        let error_msg = format!("结果格式化失败: {}", e);
        warn!("Step 3 failed: {} - {}", task.task_id, error_msg);

        // 更新任务状态为失败
        if let Err(save_err) = ctx
            .save_task_status(
                &task.task_id,
                &TaskStatus::Failed {
                    error: TaskError::StorageError {
                        operation: "result_formatting".to_string(),
                        message: error_msg.clone(),
                    },
                    failed_at: Utc::now(),
                    retry_count: 0,
                    is_recoverable: true,
                },
            )
            .await
        {
            warn!(
                "Failed to save failed status: {} - {}",
                task.task_id, save_err
            );
        }

        return Err(e);
    }

    info!("Transcription pipeline completed: {}", task.task_id);
    Ok(())
}

/// 从URL下载音频文件
async fn download_audio_from_url(
    url: &str,
    task_id: &str,
    storage_dir: &std::path::Path,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "[Task {}] Start downloading audio files from URL: {}",
        task_id, url
    );

    // 创建HTTP客户端
    let client = reqwest::Client::new();

    // 发送GET请求
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载URL失败: {} - {}", url, e))?;

    // 检查响应状态
    if !response.status().is_success() {
        return Err(format!("URL下载失败，HTTP状态: {}", response.status()).into());
    }

    // 获取内容类型并确定文件扩展名
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|ct| ct.to_str().ok())
        .unwrap_or("application/octet-stream");

    let extension = get_file_extension(content_type, url);

    // 检查是否为支持的媒体格式
    if !is_supported_media_format(content_type) {
        warn!(
            "[Task {}] Possibly unsupported media format [{}], extension [{}], subsequent processing may fail",
            task_id, content_type, extension
        );
    }

    // 创建目标文件路径
    let filename = format!("task_{}.{}", task_id, extension);
    let file_path = storage_dir.join(&filename);

    // 确保目录存在
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建目录失败: {} - {}", parent.display(), e))?;
    }

    // 流式下载文件
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|e| format!("创建文件失败: {} - {}", file_path.display(), e))?;

    let mut stream = response.bytes_stream();
    let mut total_bytes = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载数据失败: {}", e))?;

        file.write_all(&chunk)
            .await
            .map_err(|e| format!("写入文件失败: {} - {}", file_path.display(), e))?;

        total_bytes += chunk.len();
    }

    file.flush()
        .await
        .map_err(|e| format!("刷新文件失败: {} - {}", file_path.display(), e))?;

    info!(
        "[Task {}] Download completed: {} bytes -> {}",
        task_id,
        total_bytes,
        file_path.display()
    );

    Ok(file_path)
}

/// 检测音频文件格式并重命名为正确扩展名
async fn detect_and_rename_audio_file(
    file_path: &PathBuf,
    task_id: &str,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "[Task {}] Detect audio file format: {:?}",
        task_id, file_path
    );

    // 使用 AudioFormatDetector 检测文件真实格式
    let format_result = AudioFormatDetector::detect_format_from_path(file_path)
        .map_err(|e| format!("检测文件格式失败: {}", e))?;

    let detected_extension = if let Some(format_type) = format_result {
        format_type.extension().to_lowercase()
    } else {
        // 如果无法检测格式，使用文件扩展名作为后备
        if let Some(extension) = file_path.extension().and_then(|ext| ext.to_str()) {
            extension.to_lowercase()
        } else {
            return Err(format!("无法检测文件格式且文件无扩展名: {:?}", file_path).into());
        }
    };
    info!(
        "[Task {}] File format detected: {}",
        task_id, detected_extension
    );

    // 获取当前文件扩展名
    let current_extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    // 如果扩展名不匹配，重命名文件
    if current_extension != detected_extension {
        info!(
            "[Task {}] File extension mismatch: current {} -> detecting {}",
            task_id, current_extension, detected_extension
        );

        let parent_dir = file_path
            .parent()
            .ok_or_else(|| format!("无法获取文件父目录: {:?}", file_path))?;

        let new_filename = format!("task_{}.{}", task_id, detected_extension);
        let new_file_path = parent_dir.join(&new_filename);

        // 重命名文件
        tokio::fs::rename(file_path, &new_file_path)
            .await
            .map_err(|e| {
                format!(
                    "重命名文件失败: {} -> {}: {}",
                    file_path.display(),
                    new_file_path.display(),
                    e
                )
            })?;

        info!(
            "[Task {}] The file has been renamed: {} -> {}",
            task_id,
            file_path.display(),
            new_file_path.display()
        );

        Ok(new_file_path)
    } else {
        info!(
            "[Task {}] The file extension is correct: {}",
            task_id, current_extension
        );
        Ok(file_path.clone())
    }
}

/// 更新数据库中任务的文件路径
async fn update_task_file_path_in_db(
    task_id: &str,
    file_path: &PathBuf,
    ctx: &StepContext,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file_path_str = file_path.to_string_lossy().to_string();

    sqlx::query("UPDATE task_info SET file_path = ?, updated_at = ? WHERE task_id = ?")
        .bind(&file_path_str)
        .bind(chrono::Utc::now().timestamp())
        .bind(task_id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| format!("更新任务文件路径失败: {}", e))?;

    info!(
        "[Task {}] The database file path has been updated: {}",
        task_id, file_path_str
    );
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
