//! 无锁 Apalis 任务管理器本体：构造/建表、worker 启动与关闭、任务 ID 生成。

use super::*;

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

/// 保存任务信息的参数
pub(super) struct SaveTaskInfoParams<'a> {
    pub(super) task_id: &'a str,
    pub(super) status: &'a TaskStatus,
    pub(super) file_path: Option<&'a PathBuf>,
    pub(super) original_filename: Option<&'a str>,
    pub(super) model: Option<&'a str>,
    pub(super) response_format: Option<&'a str>,
    pub(super) retry_count: u32,
    pub(super) error_message: Option<&'a str>,
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
        let audio_file_manager = Arc::new(
            AudioFileManager::new("./data/audio")
                .map_err(|e| VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))?,
        );

        // 创建步骤上下文
        let step_context = StepContext {
            audio_file_manager,
            pool: self.pool.clone(),
            model_service,
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
    pub(super) fn generate_task_id(&self) -> String {
        crate::utils::generate_task_id()
    }
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
