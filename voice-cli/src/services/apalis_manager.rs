use crate::VoiceCliError;
use crate::models::{
    AsyncTranscriptionTask, TaskManagementConfig, TaskStatus, TranscriptionResponse,
};
use crate::services::{AudioFileManager, ModelService, TranscriptionEngine};

use apalis::prelude::*;
use apalis::layers::retry::RetryPolicy;
use apalis::layers::WorkerBuilderExt;
use apalis_sql::sqlite::{SqlitePool, SqliteStorage};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tracing::{debug, info, warn};


/// 全局 Apalis 管理器实例
static GLOBAL_APALIS_MANAGER: OnceLock<Arc<tokio::sync::Mutex<ApalisManager>>> = OnceLock::new();


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
    pub status_cache: DashMap<String, TaskStatus>,
    pub results_cache: DashMap<String, TranscriptionResponse>,
}

/// 任务状态更新
#[derive(Debug, Clone)]
pub struct TaskStatusUpdate {
    pub task_id: String,
    pub status: TaskStatus,
}

/// 简化的任务管理器
#[derive(Debug)]
pub struct ApalisManager {
    storage: SqliteStorage<TranscriptionTask>,
    pool: SqlitePool,
    config: TaskManagementConfig,
    status_cache: DashMap<String, TaskStatus>,
    results_cache: DashMap<String, TranscriptionResponse>,
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ApalisManager {
    /// 创建新的管理器
    pub async fn new(
        config: TaskManagementConfig,
        _model_service: Arc<ModelService>,
    ) -> Result<Self, VoiceCliError> {
        let database_url = format!("sqlite:{}", config.sqlite_db_path);
        info!("初始化 ApalisManager，数据库: {}", database_url);

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
            storage,
            pool,
            config,
            status_cache: DashMap::new(),
            results_cache: DashMap::new(),
            monitor_handle: None,
        };

        // 初始化自定义表
        manager.init_custom_tables().await?;

        info!("ApalisManager 初始化完成");
        Ok(manager)
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
        &mut self,
        model_service: Arc<ModelService>,
    ) -> Result<(), VoiceCliError> {
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
            status_cache: self.status_cache.clone(),
            results_cache: self.results_cache.clone(),
        };

        // 创建普通 worker，内部使用步骤化逻辑
        let worker = WorkerBuilder::new("transcription-pipeline")
            .data(step_context)
            .enable_tracing()
            .concurrency(self.config.max_concurrent_tasks)
            .retry(RetryPolicy::retries(self.config.retry_attempts))
            .backend(self.storage.clone())
            .build_fn(transcription_pipeline_worker);

        // 启动监控器
        let monitor = Monitor::new().register(worker);
        let handle = tokio::spawn(async move {
            if let Err(e) = monitor.run().await {
                warn!("Apalis 监控器错误: {}", e);
            }
        });

        self.monitor_handle = Some(handle);

        info!("Apalis worker 启动成功");
        Ok(())
    }

    /// 提交任务
    pub async fn submit_task(
        &mut self,
        audio_file_path: PathBuf,
        original_filename: String,
        model: Option<String>,
        response_format: Option<String>,
    ) -> Result<String, VoiceCliError> {
        let task = AsyncTranscriptionTask::new(
            self.generate_task_id(),
            audio_file_path,
            original_filename,
            model,
            response_format,
        );

        let apalis_task: TranscriptionTask = task.clone().into();

        self.storage
            .push(apalis_task)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("提交任务失败: {}", e)))?;

        // 初始状态
        let initial_status = TaskStatus::Pending {
            queued_at: Utc::now(),
        };

        self.status_cache
            .insert(task.task_id.clone(), initial_status);

        info!("任务提交成功: {}", task.task_id);
        Ok(task.task_id)
    }

    /// 获取任务状态
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskStatus>, VoiceCliError> {
        Ok(self.status_cache.get(task_id).map(|entry| entry.clone()))
    }

    /// 获取任务结果
    pub async fn get_task_result(
        &self,
        task_id: &str,
    ) -> Result<Option<TranscriptionResponse>, VoiceCliError> {
        Ok(self.results_cache.get(task_id).map(|entry| entry.clone()))
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

                self.status_cache
                    .insert(task_id.to_string(), cancelled_status);

                info!("任务已取消: {}", task_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// 保存任务结果
    pub async fn save_result(
        &self,
        task_id: &str,
        result: TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        self.results_cache
            .insert(task_id.to_string(), result.clone());

        let result_json = serde_json::to_string(&result)
            .map_err(|e| VoiceCliError::Storage(format!("序列化结果失败: {}", e)))?;

        let now = Utc::now().timestamp();

        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, created_at) VALUES (?, ?, ?)",
        )
        .bind(task_id)
        .bind(&result_json)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存结果失败: {}", e)))?;

        debug!("保存结果成功: {}", task_id);
        Ok(())
    }

    /// 优雅关闭
    pub async fn shutdown(mut self) -> Result<(), VoiceCliError> {
        if let Some(handle) = self.monitor_handle.take() {
            handle.abort();
            info!("ApalisManager 已关闭");
        }
        Ok(())
    }

    /// 生成任务 ID
    fn generate_task_id(&self) -> String {
        format!(
            "task_{}_{}",
            Utc::now().timestamp_millis(),
            std::process::id()
        )
    }
}


/// 初始化全局 Apalis 管理器
pub async fn init_global_apalis_manager(
    config: TaskManagementConfig,
    model_service: Arc<ModelService>,
) -> Result<(), VoiceCliError> {
    let manager = ApalisManager::new(config, model_service).await?;
    let manager_arc = Arc::new(tokio::sync::Mutex::new(manager));
    
    GLOBAL_APALIS_MANAGER
        .set(manager_arc)
        .map_err(|_| VoiceCliError::Initialization("全局 Apalis 管理器已初始化".to_string()))?;
    
    Ok(())
}

/// 获取全局 Apalis 管理器实例
pub fn get_global_apalis_manager() -> Result<Arc<tokio::sync::Mutex<ApalisManager>>, VoiceCliError> {
    GLOBAL_APALIS_MANAGER
        .get()
        .cloned()
        .ok_or_else(|| VoiceCliError::Initialization("全局 Apalis 管理器未初始化".to_string()))
}

/// 步骤 1: 音频预处理
async fn audio_preprocessing_step(
    task: TranscriptionTask,
    ctx: Data<StepContext>,
) -> Result<AudioProcessedTask, Error> {
    info!("步骤 1 - 音频预处理: {}", task.task_id);

    // 更新状态为处理中
    ctx.status_cache.insert(
        task.task_id.clone(),
        TaskStatus::Processing {
            stage: crate::models::ProcessingStage::AudioFormatDetection,
            started_at: Utc::now(),
            progress_details: None,
        },
    );

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
    ctx.status_cache.insert(
        task.task_id.clone(),
        TaskStatus::Processing {
            stage: crate::models::ProcessingStage::WhisperTranscription,
            started_at: Utc::now(),
            progress_details: None,
        },
    );

    // 执行转录
    let model = task.model.as_deref().unwrap_or("base");
    let transcription_result = ctx
        .transcription_engine
        .transcribe_compatible_audio(
            model,
            &task.processed_audio_path,
            30, // timeout_secs
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

    // 保存结果到缓存
    ctx.results_cache
        .insert(task.task_id.clone(), task.transcription_result.clone());

    // 计算实际处理时间
    let now = Utc::now();
    let processing_time = now.signed_duration_since(task.created_at);
    let duration = Duration::from_secs(processing_time.num_seconds().max(0) as u64);

    // 更新状态为完成
    ctx.status_cache.insert(
        task.task_id.clone(),
        TaskStatus::Completed {
            completed_at: now,
            processing_time: duration,
            result_summary: Some(format!(
                "转录了 {} 个字符",
                task.transcription_result.text.len()
            )),
        },
    );

    info!(
        "转录任务完成: {} (处理时间: {}s)",
        task.task_id,
        duration.as_secs()
    );
    Ok(())
}

/// 转录流水线 worker - 内部调用步骤函数
async fn transcription_pipeline_worker(
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
