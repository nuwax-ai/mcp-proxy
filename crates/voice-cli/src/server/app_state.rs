use crate::VoiceCliError;
use crate::models::Config;
use crate::services::TranscriptionTask;
use crate::services::{LockFreeApalisManager, ModelService};
use apalis_sql::sqlite::SqliteStorage;
use std::sync::Arc;
use std::time::SystemTime;
use tracing::info;

/// 简化的应用状态
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub model_service: Arc<ModelService>,
    pub apalis_storage: Option<SqliteStorage<TranscriptionTask>>,
    pub start_time: SystemTime,
}

impl AppState {
    /// 创建新的应用状态
    pub async fn new(config: Arc<Config>) -> Result<Self, VoiceCliError> {
        let model_service = Arc::new(ModelService::new((*config).clone()));

        // 初始化无锁 Apalis 管理器
        info!("Initializing the Lock-Free Apalis Task Manager");
        let (manager, storage) =
            LockFreeApalisManager::new(config.task_management.clone(), model_service.clone())
                .await?;

        // 启动 worker
        manager
            .start_worker(storage.clone(), model_service.clone())
            .await?;

        let apalis_storage = Some(storage);

        Ok(Self {
            config,
            model_service,
            apalis_storage,
            start_time: SystemTime::now(),
        })
    }

    /// 优雅关闭
    pub async fn shutdown(self) {
        info!("Close application state");
        // Apalis 管理器会在 Drop 时自动清理
        info!("Application status closed completed");
    }
}
