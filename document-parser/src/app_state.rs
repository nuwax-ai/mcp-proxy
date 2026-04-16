use crate::config::AppConfig;
use crate::error::AppError;
use crate::parsers::DualEngineParser;
use crate::processors::{MarkdownProcessor, MarkdownProcessorConfig};
use crate::services::{
    DocumentService, DocumentTaskProcessor, StorageService, TaskQueueService, TaskService,
};
use oss_client::OssClientTrait;
use sled::Db;
use std::sync::Arc;

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: Arc<Db>,
    pub document_service: Arc<DocumentService>,
    pub task_service: Arc<TaskService>,
    /// 公有OSS客户端
    pub oss_client: Option<Arc<dyn OssClientTrait + Send + Sync>>,
    /// 私有OSS客户端
    pub private_oss_client: Option<Arc<dyn OssClientTrait + Send + Sync>>,
    pub storage_service: Arc<StorageService>,
    pub task_queue: Arc<TaskQueueService>,
}

impl AppState {
    /// 创建新的应用状态
    pub async fn new(config: AppConfig) -> Result<Self, AppError> {
        // 初始化数据库
        let db = Self::init_database(&config).await?;
        let db_arc = Arc::new(db);

        // 初始化存储服务
        let storage_service = Arc::new(StorageService::new(db_arc.clone())?);

        // 初始化任务服务
        let task_service = Arc::new(TaskService::new(db_arc.clone())?);

        // 使用 config.storage.oss 的配置初始化公有与私有 OSS 客户端
        let public_oss_config = oss_client::OssConfig::new(
            config.storage.oss.endpoint.clone(),
            config.storage.oss.public_bucket.clone(),
            config.storage.oss.access_key_id.clone(),
            config.storage.oss.access_key_secret.clone(),
            config.storage.oss.region.clone(),
            config.storage.oss.upload_directory.clone(),
        );
        let private_oss_config = oss_client::OssConfig::new(
            config.storage.oss.endpoint.clone(),
            config.storage.oss.private_bucket.clone(),
            config.storage.oss.access_key_id.clone(),
            config.storage.oss.access_key_secret.clone(),
            config.storage.oss.region.clone(),
            config.storage.oss.upload_directory.clone(),
        );

        // 初始化公有OSS客户端（默认可用）
        let public_oss_client = oss_client::PublicOssClient::new(public_oss_config)
            .map_err(|e| AppError::Oss(e.to_string()))?;
        let oss_client: Option<Arc<dyn OssClientTrait + Send + Sync>> =
            Some(Arc::new(public_oss_client));

        // 初始化私有OSS客户端（失败不致命，记录警告）
        let private_oss_client: Option<Arc<dyn OssClientTrait + Send + Sync>> =
            match oss_client::PrivateOssClient::new(private_oss_config) {
                Ok(client) => Some(Arc::new(client)),
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize private OSS client, private client will be skipped: {}",
                        e
                    );
                    None
                }
            };

        // 初始化解析器 - 优先使用自动检测虚拟环境，回退到配置
        let dual_parser = match DualEngineParser::with_auto_venv_detection() {
            Ok(parser) => {
                tracing::info!(
                    "Initialize the parser using an automatically detected virtual environment"
                );
                parser
            }
            Err(e) => {
                tracing::warn!(
                    "Automatic detection of virtual environment failed and fell back to configuration: {}",
                    e
                );
                DualEngineParser::with_timeout(
                    &config.mineru,
                    &config.markitdown,
                    config.document_parser.processing_timeout,
                )
            }
        };

        // 初始化Markdown处理器
        let processor_config = MarkdownProcessorConfig::with_global_config();
        let markdown_processor = MarkdownProcessor::new(processor_config, None);

        // 初始化文档服务
        let document_service_config =
            crate::services::DocumentServiceConfig::from_app_config(&config);
        let document_service = Arc::new(DocumentService::with_config(
            dual_parser,
            markdown_processor,
            task_service.clone(),
            oss_client.clone(),
            document_service_config,
        ));

        // 初始化任务队列（使用配置中的并发和队列大小）
        let mut task_queue = TaskQueueService::with_config(
            task_service.clone(),
            crate::services::QueueConfig {
                max_concurrent_tasks: config.document_parser.max_concurrent,
                max_queue_size: config.document_parser.queue_size,
                task_timeout: std::time::Duration::from_secs(
                    config.document_parser.processing_timeout as u64,
                ),
                backpressure_threshold: 0.8,
                retry_base_delay: std::time::Duration::from_secs(1),
                retry_max_delay: std::time::Duration::from_secs(60),
                metrics_update_interval: std::time::Duration::from_secs(5),
                health_check_interval: std::time::Duration::from_secs(30),
            },
        );

        // 启动 worker 池
        let processor = Arc::new(DocumentTaskProcessor::new(
            document_service.clone(),
            task_service.clone(),
        ));
        task_queue
            .start(processor)
            .await
            .map_err(|e| crate::error::AppError::Internal(format!("启动任务队列失败: {e}")))?;
        let task_queue = Arc::new(task_queue);

        Ok(Self {
            config: Arc::new(config),
            db: db_arc,
            document_service,
            task_service,
            oss_client,
            private_oss_client,
            storage_service,
            task_queue,
        })
    }

    /// 初始化数据库
    async fn init_database(config: &AppConfig) -> Result<Db, AppError> {
        // 确保数据目录存在
        let db_path = &config.storage.sled.path;
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AppError::Config(format!("无法创建数据库目录: {e}")))?;
            }
        }

        // 打开数据库
        let db =
            sled::open(db_path).map_err(|e| AppError::Database(format!("无法打开数据库: {e}")))?;

        // 设置缓存容量（Sled 0.34版本不支持set_cache_capacity方法）
        // db.set_cache_capacity(config.storage.sled.cache_capacity);

        Ok(db)
    }

    /// 获取配置引用
    pub fn get_config(&self) -> &AppConfig {
        &self.config
    }

    /// 获取数据库引用
    pub fn get_db(&self) -> &Db {
        &self.db
    }

    /// 健康检查
    pub async fn health_check(&self) -> Result<(), AppError> {
        // 检查数据库连接
        self.db
            .flush()
            .map_err(|e| AppError::Database(format!("数据库健康检查失败: {e}")))?;

        // 检查配置
        if self.config.server.port == 0 {
            return Err(AppError::Config("服务器端口配置无效".to_string()));
        }

        Ok(())
    }

    /// 清理过期数据
    pub async fn cleanup_expired_data(&self) -> Result<usize, AppError> {
        let mut cleaned_count = 0;
        let _now = chrono::Utc::now();

        // 清理过期的任务数据
        let tasks_tree = self
            .db
            .open_tree("tasks")
            .map_err(|e| AppError::Database(format!("无法打开任务树: {e}")))?;

        let mut to_remove = Vec::new();
        let mut expired_tasks = Vec::new();

        for result in tasks_tree.iter() {
            match result {
                Ok((key, value)) => {
                    if let Ok(task_data) =
                        serde_json::from_slice::<crate::models::DocumentTask>(&value)
                    {
                        if task_data.is_expired() {
                            to_remove.push(key);
                            expired_tasks.push(task_data);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Error reading task data: {e}");
                }
            }
        }

        // 删除过期数据并清理相关文件
        for (i, key) in to_remove.iter().enumerate() {
            // 清理任务相关的文件
            if let Some(task) = expired_tasks.get(i) {
                self.cleanup_task_files(task).await;
            }

            if let Err(e) = tasks_tree.remove(key) {
                log::warn!("Error deleting expired tasks: {e}");
            } else {
                cleaned_count += 1;
            }
        }

        log::info!("Cleaned up {cleaned_count} expired data");
        Ok(cleaned_count)
    }

    /// 清理任务相关的临时文件
    async fn cleanup_task_files(&self, task: &crate::models::DocumentTask) {
        // 清理基于 taskId 的临时文件
        if let Some(source_path) = &task.source_path {
            // 如果是基于 taskId 的文件路径，进行清理
            if source_path.contains(&task.id) {
                if let Err(e) = tokio::fs::remove_file(source_path).await {
                    log::warn!(
                        "Cleanup task {}'s temporary files failed: {} - {}",
                        task.id,
                        source_path,
                        e
                    );
                } else {
                    log::info!(
                        "Cleaned temporary files of task {}: {}",
                        task.id,
                        source_path
                    );
                }
            }
        }

        // URL 任务不清理基于 URL 的路径（source_url），仅清理下载到本地的基于 taskId 的临时文件

        // 清理可能的工作目录（基于 taskId 的目录）
        let temp_dir = std::env::temp_dir();
        let task_work_dir = temp_dir.join(format!("document_parser_{}", task.id));
        if task_work_dir.exists() {
            if let Err(e) = tokio::fs::remove_dir_all(&task_work_dir).await {
                log::warn!(
                    "Cleanup task {}'s working directory failed: {} - {}",
                    task.id,
                    task_work_dir.display(),
                    e
                );
            } else {
                log::info!(
                    "Cleaned working directory of task {}: {}",
                    task.id,
                    task_work_dir.display()
                );
            }
        }

        // 清理基于 taskId 命名的临时文件（格式：task_{taskId}_*）
        let temp_dir_path = std::env::temp_dir();
        if let Ok(entries) = std::fs::read_dir(&temp_dir_path) {
            for entry in entries.flatten() {
                if let Some(filename) = entry.file_name().to_str() {
                    if filename.starts_with(&format!("task_{}_", task.id)) {
                        if let Err(e) = tokio::fs::remove_file(entry.path()).await {
                            log::warn!(
                                "Cleanup task {}'s temporary files failed: {} - {}",
                                task.id,
                                entry.path().display(),
                                e
                            );
                        } else {
                            log::info!(
                                "Cleaned temporary files of task {}: {}",
                                task.id,
                                entry.path().display()
                            );
                        }
                    }
                }
            }
        }
    }
}
