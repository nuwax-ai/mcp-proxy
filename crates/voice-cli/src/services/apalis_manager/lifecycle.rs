//! 任务生命周期控制：取消、重试（重新入队）、彻底删除。

use super::*;

impl LockFreeApalisManager {
    /// 取消任务
    pub async fn cancel_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        let current_status = self.get_task_status(task_id).await?;

        match current_status {
            Some(TaskStatus::Pending { .. }) | Some(TaskStatus::Processing { .. }) => {
                let cancelled_status = TaskStatus::Cancelled {
                    cancelled_at: Utc::now(),
                    reason: Some("用户取消".to_string()),
                };

                self.save_task_status(task_id, &cancelled_status).await?;

                info!("Task canceled: {}", task_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// 重试任务
    pub async fn retry_task(
        &self,
        storage: &mut SqliteStorage<TranscriptionTask>,
        task_id: &str,
    ) -> Result<bool, VoiceCliError> {
        let current_status = self.get_task_status(task_id).await?;

        match current_status {
            Some(TaskStatus::Failed { .. }) | Some(TaskStatus::Cancelled { .. }) => {
                // 查询我们自己的 task_info 表中存储的原始任务数据
                type TaskDataRow = (
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                    Option<String>,
                );
                let task_data: Option<TaskDataRow> = sqlx::query_as(
                    "SELECT file_path, original_filename, model, response_format, error_message FROM task_info WHERE task_id = ?"
                )
                .bind(task_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| VoiceCliError::Storage(format!("查询任务数据失败: {}", e)))?;

                if let Some((
                    file_path,
                    original_filename,
                    model,
                    response_format,
                    _error_message,
                )) = task_data
                {
                    if let Some(file_path_str) = file_path {
                        let audio_file_path = PathBuf::from(file_path_str);

                        // 检查文件是否仍然存在
                        if audio_file_path.exists() {
                            // 重新提交任务到 Apalis 队列
                            let result = self
                                .submit_task(
                                    storage,
                                    audio_file_path,
                                    original_filename.unwrap_or_else(|| "unknown".to_string()),
                                    model,
                                    response_format,
                                    // 重试/恢复场景：task_info 表未持久化 language/initial_prompt，
                                    // 重新提交时丢失原参数（回退自动检测）。
                                    None,
                                    None,
                                )
                                .await;

                            match result {
                                Ok(new_task_id) => {
                                    info!(
                                        "The task has been resubmitted: {} -> {}",
                                        task_id, new_task_id
                                    );
                                    Ok(true)
                                }
                                Err(e) => {
                                    warn!("Failed to resubmit task: {} - {}", task_id, e);
                                    Ok(false)
                                }
                            }
                        } else {
                            warn!(
                                "The task audio file does not exist and cannot be retried: {}",
                                task_id
                            );
                            Ok(false)
                        }
                    } else {
                        warn!(
                            "The task file path does not exist and cannot be retried: {}",
                            task_id
                        );
                        Ok(false)
                    }
                } else {
                    warn!(
                        "Task data does not exist and cannot be retried: {}",
                        task_id
                    );
                    Ok(false)
                }
            }
            Some(TaskStatus::Pending { .. }) | Some(TaskStatus::Processing { .. }) => {
                warn!(
                    "The task is being processed and cannot be retried: {}",
                    task_id
                );
                Ok(false)
            }
            Some(TaskStatus::Completed { .. }) => {
                warn!("Task completed and cannot be retried: {}", task_id);
                Ok(false)
            }
            None => {
                warn!("The task does not exist and cannot be retried: {}", task_id);
                Ok(false)
            }
        }
    }

    /// 删除任务（彻底删除任务数据和状态）
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        // 从我们自己的表中删除任务数据（不操作 apalis.jobs 表）
        let mut deleted = false;

        // 删除任务状态
        let status_result = sqlx::query("DELETE FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务状态失败: {}", e)))?;

        if status_result.rows_affected() > 0 {
            deleted = true;
            info!("Successfully deleted the status record of task {}", task_id);
        }

        // 删除任务结果
        let result_result = sqlx::query("DELETE FROM task_results WHERE task_id = ?")
            .bind(task_id)
            .execute(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("删除任务结果失败: {}", e)))?;

        if result_result.rows_affected() > 0 {
            deleted = true;
            info!("Successfully deleted the result record of task {}", task_id);
        }

        info!(
            "Task deletion operation: {} -> {}",
            task_id,
            if deleted { "success" } else { "task not found" }
        );
        Ok(deleted)
    }

    /// 检查 worker 是否运行
    pub fn is_worker_running(&self) -> bool {
        self.worker_running.load(Ordering::Acquire)
    }
}
