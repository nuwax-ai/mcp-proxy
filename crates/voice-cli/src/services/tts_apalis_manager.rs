//! TTS 异步任务管理（apalis + SQLite），镜像 STT `LockFreeApalisManager` 但更精简。
//!
//! 设计：
//! - **独立 DB**（`./data/tts_tasks.db`）：与 STT 任务隔离，避免 apalis storage 表冲突
//! - `tts_task_info` 自定义表：存任务状态 JSON + 音频文件路径（apalis 的 Job 表只存待办）
//! - worker `tts_pipeline_worker`：spawn_blocking 合成 → 落盘 `./data/tts/tts_{id}.{ext}` → 更新状态
//! - 重启恢复：apalis SQLite storage 持久化 pending 任务，worker 启动时自动重试
//!
//! 并发模型：`max_concurrent_tasks` 来自 `TaskManagementConfig`（与 STT 共享配置）。
//! 合成走 `spawn_blocking`（sherpa-onnx 同步 C 调用），不阻塞 tokio reactor。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use apalis::layers::WorkerBuilderExt;
use apalis::layers::retry::RetryPolicy;
use apalis::prelude::*;
use apalis_sql::sqlite::SqliteStorage;
use chrono::Utc;
use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;
use tracing::{debug, info, warn};

use crate::VoiceCliError;
use crate::models::tts::TtsProcessingStage;
use crate::models::{TaskManagementConfig, TtsConfig, TtsTask, TtsTaskError, TtsTaskStatus};
use crate::tts::{AudioFormat, TtsModelService, TtsOptions};

/// TTS 任务 DB 路径（独立于 STT 的 `tasks.db`，隔离 apalis storage）。
const TTS_DB_PATH: &str = "./data/tts_tasks.db";

/// worker 注入的共享上下文。
#[derive(Clone)]
pub struct TtsStepContext {
    pub pool: sqlx::SqlitePool,
    pub tts_config: TtsConfig,
    pub tts_model_service: Arc<TtsModelService>,
}

impl TtsStepContext {
    /// 保存任务状态（INSERT OR REPLACE，状态以 JSON 存）。
    async fn save_task_status(&self, task_id: &str, status: &TtsTaskStatus) -> Result<(), Error> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        let now = Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO tts_task_info (task_id, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(status_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;
        Ok(())
    }
}

/// 全局 TTS apalis 管理器实例。
static GLOBAL_TTS_APALIS_MANAGER: OnceLock<Arc<TtsApalisManager>> = OnceLock::new();

/// TTS apalis 管理器（独立 worker + 独立 SQLite pool）。
pub struct TtsApalisManager {
    pool: sqlx::SqlitePool,
    config: TaskManagementConfig,
    monitor_handle: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    worker_running: AtomicBool,
}

impl std::fmt::Debug for TtsApalisManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TtsApalisManager")
            .field(
                "worker_running",
                &self.worker_running.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl TtsApalisManager {
    /// 创建管理器，返回 `(TtsApalisManager, SqliteStorage<TtsTask>)`。
    pub async fn new(
        config: TaskManagementConfig,
    ) -> Result<(Self, SqliteStorage<TtsTask>), VoiceCliError> {
        // 确保 DB 目录 + 空文件
        let db_path = std::path::Path::new(TTS_DB_PATH);
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| VoiceCliError::Storage(format!("创建 TTS DB 目录失败: {e}")))?;
        }
        if !db_path.exists() {
            std::fs::File::create(db_path)
                .map_err(|e| VoiceCliError::Storage(format!("创建 TTS DB 文件失败: {e}")))?;
        }

        let database_url = format!("sqlite://{TTS_DB_PATH}");
        info!(%database_url, "Initialize TtsApalisManager");
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("连接 TTS DB 失败: {e}")))?;

        SqliteStorage::setup(&pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("设置 apalis storage 失败: {e}")))?;
        let storage = SqliteStorage::new(pool.clone());

        let manager = Self {
            pool,
            config,
            monitor_handle: Arc::new(tokio::sync::Mutex::new(None)),
            worker_running: AtomicBool::new(false),
        };
        manager.init_custom_tables().await?;
        info!("TtsApalisManager initialized");
        Ok((manager, storage))
    }

    async fn init_custom_tables(&self) -> Result<(), VoiceCliError> {
        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS tts_task_info (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("创建 tts_task_info 失败: {e}")))?;
        Ok(())
    }

    /// 启动 worker。
    pub async fn start_worker(
        &self,
        storage: SqliteStorage<TtsTask>,
        tts_config: TtsConfig,
        tts_model_service: Arc<TtsModelService>,
    ) -> Result<(), VoiceCliError> {
        if self.worker_running.load(Ordering::Acquire) {
            return Ok(());
        }
        let ctx = TtsStepContext {
            pool: self.pool.clone(),
            tts_config,
            tts_model_service,
        };
        info!(
            concurrency = self.config.max_concurrent_tasks,
            retry = self.config.retry_attempts,
            "Creating TTS apalis worker"
        );
        let worker = WorkerBuilder::new("tts-pipeline")
            .data(ctx)
            .enable_tracing()
            .concurrency(self.config.max_concurrent_tasks)
            .retry(RetryPolicy::retries(self.config.retry_attempts))
            .backend(storage.clone())
            .build_fn(tts_pipeline_worker);

        let monitor = Monitor::new().register(worker);
        let handle = tokio::spawn(async move {
            info!("TTS apalis monitor running");
            if let Err(e) = monitor.run().await {
                warn!("TTS apalis monitor error: {e}");
            }
        });
        self.worker_running.store(true, Ordering::Release);
        *self.monitor_handle.lock().await = Some(handle);
        // 启动过期任务清理调度器（对齐 STT；防 ./data/tts/ 无限增长）
        if let Err(e) = self.start_cleanup_scheduler().await {
            warn!("TTS cleanup scheduler 启动失败: {e}");
        }
        info!("TTS apalis worker started");
        Ok(())
    }

    /// 提交 TTS 任务。
    ///
    /// 顺序：**先写 Pending 行，再 storage.push**。若反过来（先 push 再写 Pending），
    /// worker 可能在 push 后立即取走任务并写 Processing，随后本方法的 Pending `INSERT OR REPLACE`
    /// 会把 Processing 覆盖回 Pending → 任务状态卡死。先写 Pending 后，worker 的 Processing
    /// 才是合法的后写覆盖。
    pub async fn submit_task(
        &self,
        storage: &mut SqliteStorage<TtsTask>,
        task: TtsTask,
    ) -> Result<String, VoiceCliError> {
        let task_id = task.task_id.clone();

        // 1. 先写 Pending 行（worker 后续的 Processing/Completed 会覆盖它，合法）
        let status = TtsTaskStatus::Pending {
            queued_at: Utc::now(),
        };
        let status_json = serde_json::to_string(&status)
            .map_err(|e| VoiceCliError::Storage(format!("序列化 TTS 状态失败: {e}")))?;
        let now = Utc::now().timestamp();
        sqlx::query(
            "INSERT OR REPLACE INTO tts_task_info (task_id, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(&task_id)
        .bind(status_json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存 TTS 初始状态失败: {e}")))?;

        // 2. 再 push 到 apalis 队列
        let push = tokio::time::timeout(Duration::from_secs(10), storage.push(task)).await;
        match push {
            Ok(Ok(_)) => debug!(%task_id, "TTS task pushed"),
            Ok(Err(e)) => {
                // push 失败：尽量回滚刚写的 Pending 行（best-effort，避免遗留卡死状态）
                let _ = sqlx::query("DELETE FROM tts_task_info WHERE task_id = ?")
                    .bind(&task_id)
                    .execute(&self.pool)
                    .await;
                return Err(VoiceCliError::Storage(format!("推送 TTS 任务失败: {e}")));
            }
            Err(_) => {
                let _ = sqlx::query("DELETE FROM tts_task_info WHERE task_id = ?")
                    .bind(&task_id)
                    .execute(&self.pool)
                    .await;
                return Err(VoiceCliError::Storage("推送 TTS 任务超时".to_string()));
            }
        }
        Ok(task_id)
    }

    /// 查询任务状态。
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TtsTaskStatus>, VoiceCliError> {
        let row = sqlx::query("SELECT status FROM tts_task_info WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询 TTS 任务状态失败: {e}")))?;
        if let Some(row) = row {
            let s: String = row
                .try_get("status")
                .map_err(|e| VoiceCliError::Storage(format!("读取 status 字段失败: {e}")))?;
            let status: TtsTaskStatus = serde_json::from_str(&s)
                .map_err(|e| VoiceCliError::Storage(format!("解析 TTS 状态失败: {e}")))?;
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// 删除任务：删 tts_task_info 行 + 关联音频文件（避免磁盘泄漏）。
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        // 先取状态（拿 audio_file_path），再删行
        let status_json: Option<String> =
            sqlx::query("SELECT status FROM tts_task_info WHERE task_id = ?")
                .bind(task_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| VoiceCliError::Storage(format!("查询 TTS 任务失败: {e}")))?
                .map(|r| r.try_get::<String, _>("status"))
                .transpose()
                .map_err(|e| VoiceCliError::Storage(format!("读取 status 失败: {e}")))?;
        // 删行（apalis 0.7 storage 无按 id 删除的稳定 API；仅清自定义表）
        let res = sqlx::query("DELETE FROM tts_task_info WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除 TTS 任务失败: {e}")))?;
        // 删音频文件（best-effort）
        if let Some(s) = status_json
            && let Ok(status) = serde_json::from_str::<TtsTaskStatus>(&s)
            && let TtsTaskStatus::Completed {
                audio_file_path, ..
            } = status
        {
            let _ = tokio::fs::remove_file(&audio_file_path).await;
        }
        Ok(res.rows_affected() > 0)
    }

    /// 清理过期任务（含音频文件）。对齐 STT 的 cleanup_expired_tasks。
    ///
    /// 删除 `tts_task_info` 中 updated_at 早于 `retention_minutes` 的行，
    /// 并删除其 Completed 状态里的音频文件。返回清理的行数。
    pub async fn cleanup_expired_tasks(
        &self,
        retention_minutes: u32,
    ) -> Result<usize, VoiceCliError> {
        let cutoff = Utc::now().timestamp() - (retention_minutes as i64) * 60;
        // 取过期行（status + audio 路径）
        let rows = sqlx::query("SELECT task_id, status FROM tts_task_info WHERE updated_at < ?")
            .bind(cutoff)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询过期 TTS 任务失败: {e}")))?;
        let count = rows.len();
        for row in rows {
            let task_id: String = row
                .try_get("task_id")
                .map_err(|e| VoiceCliError::Storage(format!("读取 task_id 失败: {e}")))?;
            if let Ok(status_json) = row.try_get::<String, _>("status")
                && let Ok(TtsTaskStatus::Completed {
                    audio_file_path, ..
                }) = serde_json::from_str::<TtsTaskStatus>(&status_json)
            {
                let _ = tokio::fs::remove_file(&audio_file_path).await;
            }
            let _ = sqlx::query("DELETE FROM tts_task_info WHERE task_id = ?")
                .bind(&task_id)
                .execute(&self.pool)
                .await;
        }
        if count > 0 {
            info!(count, retention_minutes, "TTS 过期任务清理完成");
        }
        Ok(count)
    }

    /// 启动定时清理调度器（每 60s 跑一次 cleanup_expired_tasks）。
    pub async fn start_cleanup_scheduler(&self) -> Result<(), VoiceCliError> {
        let pool = self.pool.clone();
        let retention = self.config.task_retention_minutes;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await; // 跳过立即触发
            loop {
                interval.tick().await;
                // 内联 cleanup 逻辑（避免 &self 跨 'static；复用 pool）
                let cutoff = Utc::now().timestamp() - (retention as i64) * 60;
                let rows = match sqlx::query(
                    "SELECT task_id, status FROM tts_task_info WHERE updated_at < ?",
                )
                .bind(cutoff)
                .fetch_all(&pool)
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("TTS cleanup 查询失败: {e}");
                        continue;
                    }
                };
                for row in rows {
                    let task_id: String = match row.try_get("task_id") {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if let Ok(status_json) = row.try_get::<String, _>("status")
                        && let Ok(TtsTaskStatus::Completed {
                            audio_file_path, ..
                        }) = serde_json::from_str::<TtsTaskStatus>(&status_json)
                    {
                        let _ = tokio::fs::remove_file(&audio_file_path).await;
                    }
                    let _ = sqlx::query("DELETE FROM tts_task_info WHERE task_id = ?")
                        .bind(&task_id)
                        .execute(&pool)
                        .await;
                }
            }
        });
        Ok(())
    }

    pub fn is_worker_running(&self) -> bool {
        self.worker_running.load(Ordering::Acquire)
    }

    /// 优雅关闭。
    pub async fn shutdown(&self) -> Result<(), VoiceCliError> {
        self.worker_running.store(false, Ordering::Release);
        if let Some(h) = self.monitor_handle.lock().await.take() {
            h.abort();
        }
        info!("TtsApalisManager closed");
        Ok(())
    }
}

/// 初始化全局 TTS apalis 管理器。
pub async fn init_global_tts_apalis_manager(
    config: TaskManagementConfig,
) -> Result<(Arc<TtsApalisManager>, SqliteStorage<TtsTask>), VoiceCliError> {
    let (manager, storage) = TtsApalisManager::new(config).await?;
    let arc = Arc::new(manager);
    let _ = GLOBAL_TTS_APALIS_MANAGER.set(arc.clone());
    Ok((arc, storage))
}

/// TTS pipeline worker：合成 → 落盘 → 更新状态。
pub async fn tts_pipeline_worker(task: TtsTask, ctx: Data<TtsStepContext>) -> Result<(), Error> {
    info!(task_id = %task.task_id, "TTS pipeline start");

    // 状态 → Processing
    let processing = TtsTaskStatus::Processing {
        stage: TtsProcessingStage::VoiceSynthesis,
        started_at: Utc::now(),
        progress_details: None,
    };
    let _ = ctx.save_task_status(&task.task_id, &processing).await;

    // 合成（spawn_blocking：sherpa-onnx 同步 C 调用）
    let started = std::time::Instant::now();
    let model_id = task.model.clone();
    let synth_result = {
        let ctx_cfg = ctx.tts_config.engine.clone();
        let model_svc = ctx.tts_model_service.clone();
        let text = task.text.clone();
        let sid = task.sid;
        let speed = task.speed;
        let length_scale = task.length_scale;
        let format_str = task.format.clone();
        let model_id_for_closure = model_id.clone();
        tokio::task::spawn_blocking(
            move || -> Result<(Vec<u8>, AudioFormat, i32, usize), String> {
                let fmt = AudioFormat::parse(&format_str);
                let opts = TtsOptions {
                    sid,
                    speed,
                    ..Default::default()
                };
                let (bytes, sr, n_samples) = crate::tts::synth_to_bytes(
                    &model_svc,
                    &model_id_for_closure,
                    &ctx_cfg,
                    &text,
                    &opts,
                    length_scale,
                    fmt,
                )
                .map_err(|e| e.to_string())?;
                Ok((bytes, fmt, sr, n_samples))
            },
        )
        .await
    };

    let (bytes, fmt, sample_rate, n_samples) = match synth_result {
        Ok(Ok(v)) => v,
        Ok(Err(msg)) => {
            warn!(task_id = %task.task_id, %msg, "TTS synth failed");
            let failed = TtsTaskStatus::Failed {
                error: TtsTaskError::SynthesisFailed {
                    model: model_id,
                    message: msg,
                },
                failed_at: Utc::now(),
            };
            let _ = ctx.save_task_status(&task.task_id, &failed).await;
            return Err(Error::from(
                Box::new(std::io::Error::other("TTS synth failed"))
                    as Box<dyn std::error::Error + Send + Sync>,
            ));
        }
        Err(join_e) => {
            warn!(task_id = %task.task_id, %join_e, "TTS spawn_blocking join failed");
            let failed = TtsTaskStatus::Failed {
                error: TtsTaskError::SynthesisFailed {
                    model: model_id,
                    message: format!("join 失败: {join_e}"),
                },
                failed_at: Utc::now(),
            };
            let _ = ctx.save_task_status(&task.task_id, &failed).await;
            return Err(Error::from(
                Box::new(std::io::Error::other("TTS join failed"))
                    as Box<dyn std::error::Error + Send + Sync>,
            ));
        }
    };

    // 落盘 ./data/tts/tts_{id}.{ext}（fs 失败也写 Failed 状态，避免任务卡 Processing）
    let ext = fmt.ext();
    let out_dir = PathBuf::from("./data/tts");
    if let Err(e) = tokio::fs::create_dir_all(&out_dir).await {
        let msg = format!("创建输出目录失败: {e}");
        warn!(task_id = %task.task_id, %msg, "TTS 落盘失败");
        let failed = TtsTaskStatus::Failed {
            error: TtsTaskError::StorageError {
                operation: "create_dir_all".to_string(),
                message: msg.clone(),
            },
            failed_at: Utc::now(),
        };
        let _ = ctx.save_task_status(&task.task_id, &failed).await;
        return Err(Error::from(
            Box::new(std::io::Error::other(msg)) as Box<dyn std::error::Error + Send + Sync>
        ));
    }
    let out_path = out_dir.join(format!("tts_{}.{ext}", task.task_id));
    if let Err(e) = tokio::fs::write(&out_path, &bytes).await {
        let msg = format!("写入音频文件失败: {e}");
        warn!(task_id = %task.task_id, %msg, "TTS 落盘失败");
        let failed = TtsTaskStatus::Failed {
            error: TtsTaskError::StorageError {
                operation: "write_audio".to_string(),
                message: msg.clone(),
            },
            failed_at: Utc::now(),
        };
        let _ = ctx.save_task_status(&task.task_id, &failed).await;
        return Err(Error::from(
            Box::new(std::io::Error::other(msg)) as Box<dyn std::error::Error + Send + Sync>
        ));
    }

    let processing_time = chrono::Duration::milliseconds(started.elapsed().as_millis() as i64);
    let duration_seconds = if sample_rate > 0 {
        n_samples as f32 / sample_rate as f32
    } else {
        0.0
    };
    let completed = TtsTaskStatus::Completed {
        completed_at: Utc::now(),
        processing_time,
        audio_file_path: out_path.to_string_lossy().into_owned(),
        file_size: bytes.len() as u64,
        duration_seconds,
    };
    let _ = ctx.save_task_status(&task.task_id, &completed).await;
    info!(task_id = %task.task_id, bytes = bytes.len(), %duration_seconds, "TTS pipeline done");
    Ok(())
}
