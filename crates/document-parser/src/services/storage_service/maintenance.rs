//! `StorageService` 的维护与备份族方法（从 storage_service.rs 拆出）：过期数据
//! 清理、内存缓存清理、数据库压缩、后台维护任务、备份与恢复。纯代码搬移。

use crate::error::AppError;
use crate::models::DocumentTask;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{TASK_PREFIX, TransactionOp};

impl super::StorageService {
    /// 清理过期数据
    pub async fn cleanup_expired_data(&self) -> Result<usize, AppError> {
        log::info!("Start cleaning expired data");

        let mut cleaned_count = 0;
        let now = SystemTime::now();

        // 清理过期任务
        let expired_tasks = self.find_expired_tasks(now).await?;

        if !expired_tasks.is_empty() {
            // 批量删除过期任务
            for chunk in expired_tasks.chunks(self.config.batch_size) {
                let result = self
                    .execute_transaction(|tx_ops| {
                        for task in chunk {
                            let task_key = format!("{}{}", TASK_PREFIX, task.id);

                            tx_ops.push(TransactionOp::Delete {
                                key: task_key.into_bytes(),
                            });

                            // 添加索引删除操作
                            self.add_index_delete_operations(task, tx_ops)?;
                        }
                        Ok(())
                    })
                    .await;

                match result {
                    Ok(()) => {
                        cleaned_count += chunk.len();

                        // 清理内存缓存
                        for task in chunk {
                            self.remove_from_memory_cache(&task.id).await;
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to delete expired tasks in batches: {e}");
                        return Err(e);
                    }
                }
            }
        }

        // 清理过期缓存
        cleaned_count += self.cleanup_expired_cache().await?;

        // 清理内存缓存
        cleaned_count += self.cleanup_memory_cache().await;

        // 更新清理时间
        self.set_last_cleanup_time(now).await?;

        // 压缩数据库
        self.compact_database().await?;

        log::info!("Cleanup completed, {cleaned_count} records deleted");
        Ok(cleaned_count)
    }

    /// 查找过期任务
    pub(super) async fn find_expired_tasks(
        &self,
        now: SystemTime,
    ) -> Result<Vec<DocumentTask>, AppError> {
        let mut expired_tasks = Vec::new();

        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {e}")))?;

            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {e}")))?;

            // 检查是否过期
            let task_created_at =
                UNIX_EPOCH + std::time::Duration::from_secs(task.created_at.timestamp() as u64);
            if let Ok(elapsed) = now.duration_since(task_created_at)
                && elapsed > self.config.retention_period
                && task.status.is_terminal()
            {
                expired_tasks.push(task);
            }
        }

        Ok(expired_tasks)
    }

    /// 清理内存缓存
    pub(super) async fn cleanup_memory_cache(&self) -> usize {
        let mut cache = self.memory_cache.write().await;
        let now = SystemTime::now();
        let mut cleaned_count = 0;

        cache.retain(|_, item| {
            if let Some(expires_at) = item.expires_at {
                if now > expires_at {
                    cleaned_count += 1;
                    false
                } else {
                    true
                }
            } else {
                true
            }
        });

        cleaned_count
    }

    /// 压缩数据库
    pub(super) async fn compact_database(&self) -> Result<(), AppError> {
        log::info!("Start compressing the database");

        // 刷新所有树
        self.tasks_tree
            .flush()
            .map_err(|e| AppError::Database(format!("刷新任务树失败: {e}")))?;

        self.index_tree
            .flush()
            .map_err(|e| AppError::Database(format!("刷新索引树失败: {e}")))?;

        self.cache_tree
            .flush()
            .map_err(|e| AppError::Database(format!("刷新缓存树失败: {e}")))?;

        self.metadata_tree
            .flush()
            .map_err(|e| AppError::Database(format!("刷新元数据树失败: {e}")))?;

        // 刷新整个数据库
        self.db
            .flush()
            .map_err(|e| AppError::Database(format!("刷新数据库失败: {e}")))?;

        log::info!("Database compression completed");
        Ok(())
    }

    /// 启动后台维护任务
    pub async fn start_maintenance_tasks(&self) -> Result<(), AppError> {
        let storage_service = self.clone_for_background();

        tokio::spawn(async move {
            let mut cleanup_interval =
                tokio::time::interval(storage_service.config.cleanup_interval);
            let mut sync_interval = tokio::time::interval(storage_service.config.sync_interval);

            loop {
                tokio::select! {
                    _ = cleanup_interval.tick() => {
                        if let Err(e) = storage_service.cleanup_expired_data().await {
                            log::error!("Periodic cleanup failed: {e}");
                        }
                    }

                    _ = sync_interval.tick() => {
                        if let Err(e) = storage_service.sync_to_disk().await {
                            log::error!("Periodic synchronization failed: {e}");
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// 同步到磁盘
    pub(super) async fn sync_to_disk(&self) -> Result<(), AppError> {
        self.compact_database().await?;

        // 更新同步时间
        {
            let mut stats = self.stats.write().await;
            stats.last_sync = Some(SystemTime::now());
        }

        Ok(())
    }

    /// 为后台任务克隆服务
    fn clone_for_background(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            tasks_tree: self.tasks_tree.clone(),
            index_tree: self.index_tree.clone(),
            cache_tree: self.cache_tree.clone(),
            metadata_tree: self.metadata_tree.clone(),
            config: self.config.clone(),
            memory_cache: Arc::clone(&self.memory_cache),
            stats: Arc::clone(&self.stats),
            transaction_counter: std::sync::atomic::AtomicU64::new(
                self.transaction_counter
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            failed_transaction_counter: std::sync::atomic::AtomicU64::new(
                self.failed_transaction_counter
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
        }
    }

    /// 备份数据
    pub async fn backup_to_path(&self, backup_path: &str) -> Result<(), AppError> {
        log::info!("Start backing up data to: {backup_path}");

        // 创建备份目录
        std::fs::create_dir_all(backup_path)
            .map_err(|e| AppError::File(format!("创建备份目录失败: {e}")))?;

        // 导出所有任务
        let mut tasks = Vec::new();
        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {e}")))?;
            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {e}")))?;
            tasks.push(task);
        }

        // 写入备份文件
        let backup_file = format!(
            "{}/tasks_backup_{}.json",
            backup_path,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        );

        let backup_data = serde_json::to_string_pretty(&tasks)
            .map_err(|e| AppError::Database(format!("序列化备份数据失败: {e}")))?;

        std::fs::write(&backup_file, backup_data)
            .map_err(|e| AppError::File(format!("写入备份文件失败: {e}")))?;

        log::info!("Backup completed: {} ({} tasks)", backup_file, tasks.len());
        Ok(())
    }

    /// 从备份恢复数据
    pub async fn restore_from_backup(&self, backup_file: &str) -> Result<usize, AppError> {
        log::info!("Restore data from backup: {backup_file}");

        let backup_data = std::fs::read_to_string(backup_file)
            .map_err(|e| AppError::File(format!("读取备份文件失败: {e}")))?;

        let tasks: Vec<DocumentTask> = serde_json::from_str(&backup_data)
            .map_err(|e| AppError::Database(format!("反序列化备份数据失败: {e}")))?;

        let mut restored_count = 0;

        for task in tasks {
            self.save_task(&task).await?;
            restored_count += 1;
        }

        log::info!("Recovery completed: {restored_count} tasks");
        Ok(restored_count)
    }
}
