//! 任务状态/结果持久化：task_info / task_results 表的读写与保存。

use super::*;

impl LockFreeApalisManager {
    /// 获取任务状态（直接从数据库查询）
    pub async fn get_task_status(
        &self,
        task_id: &str,
    ) -> Result<Option<TaskStatus>, VoiceCliError> {
        // 直接从数据库查询
        let row = sqlx::query("SELECT status FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务状态失败: {}", e)))?;

        if let Some(row) = row {
            let status_json: String = row
                .try_get("status")
                .map_err(|e| VoiceCliError::Storage(format!("获取状态字段失败: {}", e)))?;
            let status: TaskStatus = serde_json::from_str(&status_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务状态失败: {}", e)))?;

            Ok(Some(status))
        } else {
            Ok(None)
        }
    }

    /// 保存任务状态
    pub(super) async fn save_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
    ) -> Result<(), VoiceCliError> {
        let status_json = serde_json::to_string(status)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务状态失败: {}", e)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, retry_count, error_message, created_at, updated_at) VALUES (?, ?, NULL, 0, NULL, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务状态失败: {}", e)))?;

        Ok(())
    }

    /// 保存任务信息（包括文件路径）
    pub(super) async fn save_task_info(
        &self,
        params: SaveTaskInfoParams<'_>,
    ) -> Result<(), VoiceCliError> {
        let SaveTaskInfoParams {
            task_id,
            status,
            file_path,
            original_filename,
            model,
            response_format,
            retry_count,
            error_message,
        } = params;
        let status_json = serde_json::to_string(status)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务状态失败: {}", e)))?;

        let file_path_str = file_path.map(|p| p.to_string_lossy().to_string());

        sqlx::query(
            "INSERT OR REPLACE INTO task_info (task_id, status, file_path, original_filename, model, response_format, retry_count, error_message, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(task_id)
        .bind(status_json)
        .bind(file_path_str)
        .bind(original_filename)
        .bind(model)
        .bind(response_format)
        .bind(retry_count as i32)
        .bind(error_message)
        .bind(Utc::now().timestamp())
        .bind(Utc::now().timestamp())
        .execute(&self.pool)
        .await
        .map_err(|e| VoiceCliError::Storage(format!("保存任务信息失败: {}", e)))?;

        Ok(())
    }

    /// 获取任务结果
    pub async fn get_task_result(
        &self,
        task_id: &str,
        output_script: crate::models::config::OutputScript,
    ) -> Result<Option<TranscriptionResponse>, VoiceCliError> {
        let row = sqlx::query("SELECT result, metadata FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务结果失败: {}", e)))?;

        if let Some(row) = row {
            let result_json: String = row
                .try_get("result")
                .map_err(|e| VoiceCliError::Storage(format!("获取结果字段失败: {}", e)))?;

            let mut result: TranscriptionResponse = serde_json::from_str(&result_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务结果失败: {}", e)))?;

            // 繁→简转换（read-time，按当前 output_script；库存繁体原样，配置可动态切繁简）
            result.text = crate::stt::convert_if_needed(&result.text, output_script);
            for seg in &mut result.segments {
                seg.text = crate::stt::convert_if_needed(&seg.text, output_script);
            }

            // 尝试获取元数据
            let metadata_json: Option<String> = row.try_get("metadata").unwrap_or(None);

            if let Some(meta_json) = metadata_json
                && let Ok(metadata) =
                    serde_json::from_str::<crate::models::request::AudioVideoMetadata>(&meta_json)
            {
                result.metadata = Some(metadata);
            }

            Ok(Some(result))
        } else {
            Ok(None)
        }
    }

    /// 保存任务结果
    pub(super) async fn save_task_result(
        &self,
        task_id: &str,
        result: &TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        let result_json = serde_json::to_string(result)
            .map_err(|e| VoiceCliError::Storage(format!("序列化任务结果失败: {}", e)))?;

        let metadata_json = result
            .metadata
            .as_ref()
            .map(|m| {
                serde_json::to_string(m)
                    .map_err(|e| VoiceCliError::Storage(format!("序列化元数据失败: {}", e)))
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
        .map_err(|e| VoiceCliError::Storage(format!("保存任务结果失败: {}", e)))?;

        Ok(())
    }

    /// 保存任务结果
    pub async fn save_result(
        &self,
        task_id: &str,
        result: TranscriptionResponse,
    ) -> Result<(), VoiceCliError> {
        self.save_task_result(task_id, &result).await?;

        debug!("Successfully saved result: {}", task_id);
        Ok(())
    }
}
