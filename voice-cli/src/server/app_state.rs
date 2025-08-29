use crate::models::Config;
use crate::services::{ApalisManager, ModelService};
use crate::VoiceCliError;
use apalis_sql::sqlite::SqliteStorage;
use crate::services::TranscriptionTask;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tracing::info;

/// 简化的应用状态
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub model_service: Arc<ModelService>,
    pub apalis_manager: Option<Arc<Mutex<ApalisManager>>>,
    pub apalis_storage: Option<SqliteStorage<TranscriptionTask>>,
    pub start_time: SystemTime,
}

impl AppState {
    /// 创建新的应用状态
    pub async fn new(config: Arc<Config>) -> Result<Self, VoiceCliError> {
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // 如果启用任务管理，初始化 Apalis 管理器
        let (apalis_manager, apalis_storage) = if config.task_management.enabled {
            info!("初始化 Apalis 任务管理器");
            let (mut manager, storage) = ApalisManager::new(
                config.task_management.clone(),
                model_service.clone(),
            ).await?;

            // 启动 worker
            manager.start_worker(storage.clone(), model_service.clone()).await?;

            (Some(Arc::new(Mutex::new(manager))), Some(storage))
        } else {
            (None, None)
        };

        Ok(Self {
            config,
            model_service,
            apalis_manager,
            apalis_storage,
            start_time: SystemTime::now(),
        })
    }

    /// 优雅关闭
    pub async fn shutdown(self) {
        info!("关闭应用状态");
        // Apalis 管理器会在 Drop 时自动清理
        info!("应用状态关闭完成");
    }
}