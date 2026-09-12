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

        // 终态守卫：当前已是 Cancelled/Completed 时不被翻写（worker 完成时把
        // 并发写入的 Cancelled 覆盖为 Completed 是取消 API 的语义破坏——
        // 客户端"取消成功又复活"；读后写窗口为毫秒级，可接受）
        {
            let row = sqlx::query("SELECT status FROM task_info WHERE task_id = ?")
                .bind(task_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(
                    |e| Error::from(Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                )?;
            if let Some(existing) = row.as_ref().and_then(|r| {
                r.try_get::<String, _>("status")
                    .ok()
                    .and_then(|s| serde_json::from_str::<TaskStatus>(&s).ok())
            }) && matches!(
                existing,
                TaskStatus::Cancelled { .. } | TaskStatus::Completed { .. }
            ) && !matches!(
                status,
                TaskStatus::Cancelled { .. } | TaskStatus::Completed { .. }
            ) {
                warn!(
                    "save_task_status skipped: task {} terminal {:?} not overwritten by {:?}",
                    task_id, existing, status
                );
                return Ok(());
            }
        }

        sqlx::query(
            // UPSERT 只更新状态列：INSERT OR REPLACE 是删整行重插，未列出的列
        //（file_path/original_filename/model/...）会被置 NULL——音频文件
        // 永不清理（cleanup 拿 NULL 只删行）+ 重试 API 恒失败（retry 读 NULL）
        "INSERT INTO task_info (task_id, status, file_path, retry_count, error_message, created_at, updated_at) VALUES (?, ?, NULL, 0, NULL, ?, ?) ON CONFLICT(task_id) DO UPDATE SET status = excluded.status, updated_at = excluded.updated_at"
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
