//! STT 转录任务管理：无锁 Apalis 管理器（SQLite 持久化）+ 三步骤流水线。
//!
//! 按职责拆分为多个子模块，本模块通过 re-export 保持对外统一的 API 面：
//! 所有 `services::apalis_manager::xxx` 路径与拆分前完全一致。
//! 子模块通过 `use super::*` 共享本模块导入（祖先私有项对后代可见）。

use crate::VoiceCliError;
use crate::models::{
    AsyncTranscriptionTask, ProcessingStage, TaskError, TaskManagementConfig, TaskStatsResponse,
    TaskStatus, TranscriptionResponse,
};
use crate::services::{AudioFileManager, AudioFormatDetector, MetadataExtractor, ModelService};
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
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

pub mod audio_source;
pub mod cleanup;
pub mod lifecycle;
pub mod manager;
pub mod pipeline;
pub mod status;
pub mod step_context;
pub mod submit;
pub mod types;

pub use manager::LockFreeApalisManager;

// 跨子模块共享的内部项（经 use super::* 对全部子模块可见）
use audio_source::{
    detect_and_rename_audio_file, download_audio_from_url, update_task_file_path_in_db,
};
use manager::SaveTaskInfoParams;
pub use pipeline::transcription_pipeline_worker;
pub use step_context::StepContext;
pub use types::*;
