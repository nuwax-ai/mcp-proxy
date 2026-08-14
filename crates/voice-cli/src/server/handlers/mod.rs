//! HTTP handlers：STT 转录（同步/异步）、任务队列管理、TTS。
//!
//! 按路由域拆分为多个子模块，本模块通过 re-export 保持对外统一的 API 面：
//! 所有 `handlers::xxx_handler` 路径（routes.rs / openapi.rs / stream 模块）
//! 与拆分前完全一致。AppState 保留在本模块，子模块经 `use super::*` 共享。

use crate::VoiceCliError;
use crate::models::config::TtsBackend;
use crate::models::{
    AsyncTaskResponse, CancelResponse, Config, DeleteResponse, HealthResponse, HttpResult,
    ModelsResponse, RetryResponse, SimpleTaskStatus, TaskStatsResponse, TaskStatus,
    TaskStatusResponse, TranscriptionResponse, TtsAsyncRequest, TtsSyncRequest, TtsTaskResponse,
    TtsTaskStatus,
};
use crate::services::{
    AudioFileManager, AudioFormatDetector, LockFreeApalisManager, MetadataExtractor, ModelService,
    TranscriptionTask, TtsApalisManager,
};
use crate::tts::{AudioFormat, TtsModelService, TtsOptions, resolve_reference};
use apalis_sql::sqlite::SqliteStorage;
use axum::extract::{Json, Multipart, Path as AxumPath, State};
use axum::response::IntoResponse;
use chrono::Utc;
use futures::TryStreamExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};
use url::Url;
use utoipa;

pub mod health;
pub mod stt_async;
pub mod stt_common;
pub mod stt_sync;
pub mod tasks;
pub mod tts;

pub use health::*;
pub use stt_async::*;
pub use stt_sync::*;
pub use tasks::*;
pub use tts::*;

pub use stt_common::UrlTranscriptionRequest;
// 跨子模块共享的内部项（sync/async handler 共用，经 use super::* 可见）
use stt_common::{extract_filename_from_url, extract_transcription_request_streaming};

#[derive(Clone, Debug)]
pub struct AppState {
    pub config: Arc<Config>,
    pub model_service: Arc<ModelService>,
    pub lock_free_apalis_manager: Arc<LockFreeApalisManager>,
    pub apalis_storage: SqliteStorage<TranscriptionTask>,
    pub audio_file_manager: Arc<AudioFileManager>,
    pub tts_model_service: Arc<TtsModelService>,
    pub tts_apalis_manager: Arc<TtsApalisManager>,
    pub tts_apalis_storage: SqliteStorage<crate::models::TtsTask>,
    pub start_time: SystemTime,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> crate::Result<Self> {
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

        let lock_free_apalis_manager = Arc::new(manager);
        let apalis_storage = storage;

        // 初始化音频文件管理器
        let audio_file_manager = Arc::new(
            AudioFileManager::new("./data/audio")
                .map_err(|e| VoiceCliError::Storage(format!("创建音频文件管理器失败: {}", e)))?,
        );

        // 初始化 TTS 模型服务（v1 不自动下载；缺模型时请求阶段返回明确错误）
        let tts_model_service =
            Arc::new(TtsModelService::new(config.tts.engine.models_dir.clone()));

        // 初始化 TTS apalis 管理器（独立 DB，路径来自 config.tts.tasks_db_path）
        info!("Initializing TTS Apalis manager");
        let (tts_apalis_manager, tts_apalis_storage) = TtsApalisManager::new(
            config.task_management.clone(),
            config.tts.tasks_db_path.clone(),
        )
        .await?;
        let tts_apalis_manager = Arc::new(tts_apalis_manager);
        tts_apalis_manager
            .start_worker(
                tts_apalis_storage.clone(),
                config.tts.clone(),
                tts_model_service.clone(),
            )
            .await?;

        Ok(Self {
            config,
            model_service,
            lock_free_apalis_manager,
            apalis_storage,
            audio_file_manager,
            tts_model_service,
            tts_apalis_manager,
            tts_apalis_storage,
            start_time: SystemTime::now(),
        })
    }

    /// 优雅关闭
    pub async fn shutdown(&self) {
        info!("Close application state");

        // 优雅关闭 Apalis 管理器
        if let Err(e) = self.lock_free_apalis_manager.shutdown().await {
            warn!("Failed to close Apalis Manager: {}", e);
        }
        // 优雅关闭 TTS apalis 管理器
        if let Err(e) = self.tts_apalis_manager.shutdown().await {
            warn!("Failed to close TTS Apalis Manager: {}", e);
        }

        info!("Application status closed completed");
    }
}
