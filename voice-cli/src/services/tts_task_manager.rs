use crate::VoiceCliError;
use crate::models::{
    TtsAsyncRequest, TtsProcessingStage, TtsProgressDetails, TtsTaskError, TtsTaskStatus,
};
use apalis::prelude::*;
use apalis_sql::sqlite::SqliteStorage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// TTS任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsTask {
    pub task_id: String,
    pub text: String,
    pub model: Option<String>,
    pub speed: f32,
    pub pitch: i32,
    pub volume: f32,
    pub format: String,
    pub created_at: DateTime<Utc>,
    pub priority: u32,
}

/// TTS任务状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsTaskState {
    pub task_id: String,
    pub status: TtsTaskStatus,
    pub updated_at: DateTime<Utc>,
}

/// TTS任务管理器
pub struct TtsTaskManager {
    storage: Arc<RwLock<SqliteStorage<TtsTask>>>,
    max_concurrent_tasks: usize,
}

impl TtsTaskManager {
    /// 创建新的TTS任务管理器
    pub async fn new(
        database_url: &str,
        max_concurrent_tasks: usize,
    ) -> Result<Self, VoiceCliError> {
        info!("初始化TTS任务管理器 - 数据库: {}", database_url);

        // 创建SQLite存储
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("连接SQLite失败: {}", e)))?;

        let storage = Arc::new(RwLock::new(SqliteStorage::new(pool)));

        // 创建任务表
        Self::create_tables_if_not_exists(&storage).await?;

        Ok(Self {
            storage,
            max_concurrent_tasks,
        })
    }

    /// 创建必要的表
    async fn create_tables_if_not_exists(
        storage: &Arc<RwLock<SqliteStorage<TtsTask>>>,
    ) -> Result<(), VoiceCliError> {
        let guard = storage.read().await;
        let pool = guard.pool();

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tts_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL UNIQUE,
                text TEXT NOT NULL,
                model TEXT,
                speed REAL NOT NULL,
                pitch INTEGER NOT NULL,
                volume REAL NOT NULL,
                format TEXT NOT NULL,
                created_at TEXT NOT NULL,
                priority INTEGER NOT NULL,
                status TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                result_path TEXT,
                file_size INTEGER,
                duration_seconds REAL,
                error_message TEXT,
                retry_count INTEGER DEFAULT 0
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("创建TTS任务表失败: {}", e)))?;

        info!("TTS任务表创建成功");
        Ok(())
    }

    /// 提交TTS任务
    pub async fn submit_task(&self, request: TtsAsyncRequest) -> Result<String, VoiceCliError> {
        let task_id = Uuid::new_v4().to_string();
        let created_at = Utc::now();

        let task = TtsTask {
            task_id: task_id.clone(),
            text: request.text.clone(),
            model: request.model.clone(),
            speed: request.speed.unwrap_or(1.0),
            pitch: request.pitch.unwrap_or(0),
            volume: request.volume.unwrap_or(1.0),
            format: request.format.unwrap_or_else(|| "mp3".to_string()),
            created_at,
            priority: request.priority.map_or(2, |p| match p {
                crate::models::tts::TaskPriority::Low => 1,
                crate::models::tts::TaskPriority::Normal => 2,
                crate::models::tts::TaskPriority::High => 3,
            }),
        };

        // 保存任务到数据库
        let guard = self.storage.read().await;
        let pool = guard.pool();

        sqlx::query(
            r#"
            INSERT INTO tts_tasks (
                task_id, text, model, speed, pitch, volume, format, 
                created_at, priority, status, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&task.task_id)
        .bind(&task.text)
        .bind(&task.model)
        .bind(task.speed)
        .bind(task.pitch)
        .bind(task.volume)
        .bind(&task.format)
        .bind(task.created_at)
        .bind(task.priority)
        .bind("pending")
        .bind(task.created_at)
        .execute(pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存TTS任务失败: {}", e)))?;

        info!("TTS任务已提交 - ID: {}", task_id);
        Ok(task_id)
    }

    /// 获取任务状态
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TtsTaskStatus>, VoiceCliError> {
        let guard = self.storage.read().await;
        let pool = guard.pool();

        let row = sqlx::query(
            "SELECT status, updated_at, result_path, file_size, duration_seconds, error_message, retry_count FROM tts_tasks WHERE task_id = ?"
        )
        .bind(task_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("查询任务状态失败: {}", e)))?;

        match row {
            Some(row) => {
                let status_str: String = row.get("status");
                let updated_at: DateTime<Utc> = row.get("updated_at");
                let result_path: Option<String> = row.get("result_path");
                let file_size: Option<i64> = row.get("file_size");
                let duration_seconds: Option<f64> = row.get("duration_seconds");
                let error_message: Option<String> = row.get("error_message");
                let retry_count: i32 = row.get("retry_count");

                let status = match status_str.as_str() {
                    "pending" => TtsTaskStatus::Pending {
                        queued_at: updated_at,
                    },
                    "processing" => TtsTaskStatus::Processing {
                        stage: TtsProcessingStage::VoiceSynthesis,
                        started_at: updated_at,
                        progress_details: Some(TtsProgressDetails {
                            current_stage: TtsProcessingStage::VoiceSynthesis,
                            stage_progress: Some(0.5),
                            estimated_remaining: Some(chrono::Duration::seconds(30)),
                            text_length: 100,
                            processed_chars: 50,
                        }),
                    },
                    "completed" => {
                        if let (Some(path), Some(size), Some(duration)) =
                            (result_path, file_size, duration_seconds)
                        {
                            TtsTaskStatus::Completed {
                                completed_at: updated_at,
                                processing_time: updated_at.signed_duration_since(updated_at), // 这里应该用创建时间
                                audio_file_path: path,
                                file_size: size as u64,
                                duration_seconds: duration as f32,
                            }
                        } else {
                            return Err(VoiceCliError::Storage(
                                "完成的任务缺少结果信息".to_string(),
                            ));
                        }
                    }
                    "failed" => {
                        let error = error_message.unwrap_or_else(|| "未知错误".to_string());
                        TtsTaskStatus::Failed {
                            error: TtsTaskError::SynthesisFailed {
                                model: "default".to_string(),
                                message: error,
                                is_recoverable: retry_count < 3,
                            },
                            failed_at: updated_at,
                            retry_count: retry_count as u32,
                            is_recoverable: retry_count < 3,
                        }
                    }
                    "cancelled" => TtsTaskStatus::Cancelled {
                        cancelled_at: updated_at,
                        reason: None,
                    },
                    _ => {
                        return Err(VoiceCliError::Storage(format!(
                            "未知的任务状态: {}",
                            status_str
                        )));
                    }
                };

                Ok(Some(status))
            }
            None => Ok(None),
        }
    }

    /// 更新任务状态
    pub async fn update_task_status(
        &self,
        task_id: &str,
        status: TtsTaskStatus,
    ) -> Result<(), VoiceCliError> {
        let guard = self.storage.read().await;
        let pool = guard.pool();
        let updated_at = Utc::now();

        let (status_str, result_path, file_size, duration_seconds, error_message) = match status {
            TtsTaskStatus::Pending { .. } => ("pending", None, None, None, None),
            TtsTaskStatus::Processing { .. } => ("processing", None, None, None, None),
            TtsTaskStatus::Completed {
                audio_file_path,
                file_size,
                duration_seconds,
                ..
            } => (
                "completed",
                Some(audio_file_path),
                Some(file_size as i64),
                Some(duration_seconds as f64),
                None,
            ),
            TtsTaskStatus::Failed { error, .. } => {
                ("failed", None, None, None, Some(error.to_string()))
            }
            TtsTaskStatus::Cancelled { .. } => ("cancelled", None, None, None, None),
        };

        sqlx::query(
            "UPDATE tts_tasks SET status = ?, updated_at = ?, result_path = ?, file_size = ?, duration_seconds = ?, error_message = ? WHERE task_id = ?"
        )
        .bind(status_str)
        .bind(updated_at)
        .bind(result_path)
        .bind(file_size)
        .bind(duration_seconds)
        .bind(error_message)
        .bind(task_id)
        .execute(pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("更新任务状态失败: {}", e)))?;

        Ok(())
    }

    /// 启动任务处理器
    pub async fn start_worker(&self) -> Result<(), VoiceCliError> {
        info!("启动TTS任务处理器");

        // TODO: 实现实际的任务处理逻辑
        // 这里应该启动一个后台worker来处理TTS任务队列

        Ok(())
    }

    /// 获取任务统计
    pub async fn get_stats(&self) -> Result<TtsTaskStats, VoiceCliError> {
        let guard = self.storage.read().await;
        let pool = guard.pool();

        let row = sqlx::query(
            r#"
            SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
                SUM(CASE WHEN status = 'processing' THEN 1 ELSE 0 END) as processing,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
                SUM(CASE WHEN status = 'cancelled' THEN 1 ELSE 0 END) as cancelled
            FROM tts_tasks
            "#,
        )
        .fetch_one(pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("获取任务统计失败: {}", e)))?;

        Ok(TtsTaskStats {
            total_tasks: row.get("total"),
            pending_tasks: row.get("pending"),
            processing_tasks: row.get("processing"),
            completed_tasks: row.get("completed"),
            failed_tasks: row.get("failed"),
            cancelled_tasks: row.get("cancelled"),
        })
    }
}

/// TTS任务统计
#[derive(Debug, Clone)]
pub struct TtsTaskStats {
    pub total_tasks: i64,
    pub pending_tasks: i64,
    pub processing_tasks: i64,
    pub completed_tasks: i64,
    pub failed_tasks: i64,
    pub cancelled_tasks: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_tts_task_manager_creation() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path().to_string_lossy().to_string();
        let db_url = format!("sqlite://{}", db_path);

        let manager = TtsTaskManager::new(&db_url, 2).await.unwrap();
        assert_eq!(manager.max_concurrent_tasks, 2);
    }

    #[tokio::test]
    async fn test_task_submission() {
        let temp_file = NamedTempFile::new().unwrap();
        let db_path = temp_file.path().to_string_lossy().to_string();
        let db_url = format!("sqlite://{}", db_path);

        let manager = TtsTaskManager::new(&db_url, 2).await.unwrap();

        let request = TtsAsyncRequest {
            text: "Hello, world!".to_string(),
            model: None,
            speed: Some(1.0),
            pitch: Some(0),
            volume: Some(1.0),
            format: Some("mp3".to_string()),
            priority: None,
        };

        let task_id = manager.submit_task(request).await.unwrap();
        assert!(!task_id.is_empty());
    }
}
