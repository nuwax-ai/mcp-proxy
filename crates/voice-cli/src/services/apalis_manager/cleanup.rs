//! 任务统计与清理：统计聚合、过期任务清理、音频文件级删除、定时清理调度器。

use super::*;

impl LockFreeApalisManager {
    /// 获取任务统计信息
    pub async fn get_tasks_stats(&self) -> Result<TaskStatsResponse, VoiceCliError> {
        let mut total_tasks = 0u32;
        let mut pending_tasks = 0u32;
        let mut processing_tasks = 0u32;
        let mut completed_tasks = 0u32;
        let mut failed_tasks = 0u32;
        let mut cancelled_tasks = 0u32;
        let mut failed_task_ids = Vec::new();
        let mut processing_times = Vec::new();

        // 从 SQLite 查询所有任务状态
        let rows = sqlx::query("SELECT task_id, status FROM task_info")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务统计失败: {}", e)))?;

        for row in rows {
            let task_id: String = row
                .try_get("task_id")
                .map_err(|e| VoiceCliError::Storage(format!("获取任务ID失败: {}", e)))?;
            let status_json: String = row
                .try_get("status")
                .map_err(|e| VoiceCliError::Storage(format!("获取状态字段失败: {}", e)))?;

            let status: TaskStatus = serde_json::from_str(&status_json)
                .map_err(|e| VoiceCliError::Storage(format!("解析任务状态失败: {}", e)))?;

            total_tasks += 1;

            match status {
                TaskStatus::Pending { .. } => {
                    pending_tasks += 1;
                }
                TaskStatus::Processing { .. } => {
                    processing_tasks += 1;
                }
                TaskStatus::Completed {
                    processing_time, ..
                } => {
                    completed_tasks += 1;
                    processing_times.push(processing_time.as_millis() as f64);
                }
                TaskStatus::Failed { .. } => {
                    failed_tasks += 1;
                    failed_task_ids.push(task_id);
                }
                TaskStatus::Cancelled { .. } => {
                    cancelled_tasks += 1;
                }
            }
        }

        // 计算平均处理时间
        let average_processing_time_ms = if !processing_times.is_empty() {
            Some(processing_times.iter().sum::<f64>() / processing_times.len() as f64)
        } else {
            None
        };

        let stats = TaskStatsResponse {
            total_tasks,
            pending_tasks,
            processing_tasks,
            completed_tasks,
            failed_tasks,
            cancelled_tasks,
            average_processing_time_ms,
            failed_task_ids,
        };

        info!(
            "Task statistics: Total {} tasks, {} completed, {} failed",
            total_tasks, completed_tasks, failed_tasks
        );

        Ok(stats)
    }

    /// 清理过期任务
    pub async fn cleanup_expired_tasks(&self) -> Result<usize, VoiceCliError> {
        let retention_minutes = self.config.task_retention_minutes;
        if retention_minutes == 0 {
            info!("The task retention minutes is 0 and cleanup is skipped");
            return Ok(0);
        }

        let cutoff_time = chrono::Utc::now() - chrono::Duration::minutes(retention_minutes as i64);
        let cutoff_timestamp = cutoff_time.timestamp();

        info!(
            "Start cleaning up expired tasks, retention minutes: {}, deadline: {}",
            retention_minutes, cutoff_time
        );

        // 获取过期任务列表
        let expired_tasks = self.get_expired_task_ids(cutoff_timestamp).await?;

        let mut cleaned_count = 0;

        for task_id in &expired_tasks {
            if let Ok(deleted) = self.delete_task_with_files(task_id).await
                && deleted
            {
                cleaned_count += 1;
            }
        }

        info!(
            "Cleanup completed: {} expired tasks in total, {} were successfully cleaned",
            expired_tasks.len(),
            cleaned_count
        );
        Ok(cleaned_count)
    }

    /// 获取过期任务ID列表
    async fn get_expired_task_ids(
        &self,
        cutoff_timestamp: i64,
    ) -> Result<Vec<String>, VoiceCliError> {
        // 添加调试日志
        info!(
            "Query expired tasks, deadline timestamp: {}",
            cutoff_timestamp
        );

        let rows = sqlx::query("SELECT task_id, updated_at FROM task_info WHERE updated_at < ?")
            .bind(cutoff_timestamp)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询过期任务失败: {}", e)))?;

        info!("Found {} expired task records", rows.len());

        let mut task_ids = Vec::new();
        for row in rows {
            if let Ok(task_id) = row.try_get::<String, _>("task_id") {
                if let Ok(updated_at) = row.try_get::<i64, _>("updated_at") {
                    info!("Expired tasks: {} (updated_at: {})", task_id, updated_at);
                }
                task_ids.push(task_id);
            }
        }

        Ok(task_ids)
    }

    /// 删除任务及其相关文件
    async fn delete_task_with_files(&self, task_id: &str) -> Result<bool, VoiceCliError> {
        info!("Start deleting the task and its files: {}", task_id);

        // 首先获取任务的文件路径信息
        if let Some(audio_file_path) = self.get_task_audio_file_path(task_id).await? {
            info!("Found task audio file: {:?}", audio_file_path);

            // 删除音频文件
            if let Err(e) = tokio::fs::remove_file(&audio_file_path).await {
                warn!(
                    "Failed to delete audio files: {} - {}",
                    audio_file_path.display(),
                    e
                );
            } else {
                info!("Audio file deleted successfully: {:?}", audio_file_path);
            }

            // 尝试删除文件所在目录（如果为空）
            if let Some(parent_dir) = audio_file_path.parent() {
                let _ = tokio::fs::remove_dir(parent_dir).await;
            }
        } else {
            info!("Mission audio file not found: {}", task_id);
        }

        // 删除数据库中的任务数据
        let result = self.delete_task(task_id).await;
        info!("Delete task database record result: {:?}", result);
        result
    }

    /// 获取任务的音频文件路径
    async fn get_task_audio_file_path(
        &self,
        task_id: &str,
    ) -> Result<Option<PathBuf>, VoiceCliError> {
        // 从 task_info 表中查询文件路径
        let row = sqlx::query("SELECT file_path FROM task_info WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| VoiceCliError::Storage(format!("查询任务文件路径失败: {}", e)))?;

        if let Some(row) = row {
            let file_path: Option<String> = row
                .try_get("file_path")
                .map_err(|e| VoiceCliError::Storage(format!("获取文件路径字段失败: {}", e)))?;

            if let Some(file_path_str) = file_path {
                let path = PathBuf::from(file_path_str);
                if path.exists() {
                    info!("Found audio file for task {}: {:?}", task_id, path);
                    return Ok(Some(path));
                } else {
                    info!(
                        "The file path for task {} exists but the file does not exist: {:?}",
                        task_id, path
                    );
                }
            }
        }

        info!("Audio file path not found for task {}", task_id);
        Ok(None)
    }

    /// 启动定时清理任务
    pub async fn start_cleanup_scheduler(&self) -> Result<(), VoiceCliError> {
        if self.config.task_retention_minutes == 0 {
            info!(
                "The number of task retention minutes is 0 and the cleanup scheduler is not started."
            );
            return Ok(());
        }

        let manager = self.clone();

        tokio::spawn(async move {
            // 初始延迟，避免立即清理
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            // 定时清理任务，默认每1分钟清理一次
            let cleanup_interval = tokio::time::Duration::from_secs(60);

            loop {
                tokio::time::sleep(cleanup_interval).await;

                match manager.cleanup_expired_tasks().await {
                    Ok(cleaned_count) => {
                        if cleaned_count > 0 {
                            info!(
                                "Scheduled cleanup completed: {} expired tasks cleaned up",
                                cleaned_count
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Scheduled cleanup task failed: {}", e);
                    }
                }
            }
        });

        info!(
            "The task cleanup scheduler is started successfully, and the number of reserved minutes is: {} minutes",
            self.config.task_retention_minutes
        );
        Ok(())
    }
}
