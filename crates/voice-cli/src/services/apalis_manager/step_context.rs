//! 流水线步骤共享上下文（音频文件管理器 / 连接池 / 模型服务）及其持久化方法。

use super::*;

/// 步骤共享上下文
#[derive(Debug, Clone)]
pub struct StepContext {
    pub audio_file_manager: Arc<AudioFileManager>,
    pub pool: sqlx::SqlitePool,
    /// STT 模型管理（transcribe-rs 引擎池的模型加载/下载）
    pub model_service: Arc<ModelService>,
}

impl StepContext {
    /// 保存任务状态到SQLite
    pub(super) async fn save_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
    ) -> Result<(), Error> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        sqlx::query(
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, retry_count, error_message, created_at, updated_at) VALUES (?, ?, NULL, 0, NULL, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        Ok(())
    }

    /// 保存任务结果到SQLite
    pub(super) async fn save_task_result(
        &self,
        task_id: &str,
        result: &TranscriptionResponse,
        metadata: &Option<crate::models::request::AudioVideoMetadata>,
    ) -> Result<(), Error> {
        let result_json = serde_json::to_string(result)
            .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        let metadata_json = metadata
            .as_ref()
            .map(|m| {
                serde_json::to_string(m).map_err(|e| {
                    Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
                })
            })
            .transpose()?;

        sqlx::query(
            "INSERT OR REPLACE INTO task_results (task_id, result, metadata, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(task_id)
        .bind(result_json)
        .bind(metadata_json)
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

        Ok(())
    }
}
