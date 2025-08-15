use std::sync::Arc;
use sled::Db;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::services::{
    DocumentService, TaskService, OssService, StorageService
};
use crate::parsers::DualEngineParser;
use crate::processors::{MarkdownProcessor, MarkdownProcessorConfig};

/// 应用状态
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: Arc<Db>,
    pub document_service: Arc<DocumentService>,
    pub task_service: Arc<TaskService>,
    pub oss_service: Option<Arc<OssService>>,
    pub storage_service: Arc<StorageService>,
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
        
        // 初始化OSS服务
        let oss_service = if config.environment == "test" {
            // 在测试环境中跳过 OSS 服务初始化
            None
        } else {
            Some(Arc::new(OssService::new(&config.storage.oss).await?))
        };
        
        // 初始化解析器 - 优先使用自动检测虚拟环境，回退到配置
        let dual_parser = match DualEngineParser::with_auto_venv_detection() {
            Ok(parser) => {
                tracing::info!("使用自动检测的虚拟环境初始化解析器");
                parser
            }
            Err(e) => {
                tracing::warn!("自动检测虚拟环境失败，回退到配置: {}", e);
                DualEngineParser::with_timeout(&config.mineru, &config.markitdown, config.document_parser.processing_timeout)
            }
        };
        
        // 初始化Markdown处理器
        let processor_config = MarkdownProcessorConfig::with_global_config();
        let markdown_processor = MarkdownProcessor::new(processor_config);
        
        // 初始化文档服务
        let document_service_config = crate::services::DocumentServiceConfig::from_app_config(&config);
        let document_service = Arc::new(DocumentService::with_config(
            dual_parser,
            markdown_processor,
            task_service.clone(),
            oss_service.clone(),
            document_service_config,
        ));
        
        Ok(Self {
            config: Arc::new(config),
            db: db_arc,
            document_service,
            task_service,
            oss_service,
            storage_service,
        })
    }

    /// 初始化数据库
    async fn init_database(config: &AppConfig) -> Result<Db, AppError> {
        // 确保数据目录存在
        let db_path = &config.storage.sled.path;
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AppError::Config(format!("无法创建数据库目录: {}", e)))?;
            }
        }

        // 打开数据库
        let db = sled::open(db_path)
            .map_err(|e| AppError::Database(format!("无法打开数据库: {}", e)))?;

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
        self.db.flush()
            .map_err(|e| AppError::Database(format!("数据库健康检查失败: {}", e)))?;

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
        let tasks_tree = self.db.open_tree("tasks")
            .map_err(|e| AppError::Database(format!("无法打开任务树: {}", e)))?;

        let mut to_remove = Vec::new();
        for result in tasks_tree.iter() {
            match result {
                Ok((key, value)) => {
                    if let Ok(task_data) = serde_json::from_slice::<crate::models::DocumentTask>(&value) {
                        if task_data.is_expired() {
                            to_remove.push(key);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("读取任务数据时出错: {}", e);
                }
            }
        }

        // 删除过期数据
        for key in to_remove {
            if let Err(e) = tasks_tree.remove(&key) {
                log::warn!("删除过期任务时出错: {}", e);
            } else {
                cleaned_count += 1;
            }
        }

        log::info!("清理了 {} 条过期数据", cleaned_count);
        Ok(cleaned_count)
    }
}
