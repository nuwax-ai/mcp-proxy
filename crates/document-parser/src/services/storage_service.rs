use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use crate::error::AppError;
use crate::models::{DocumentFormat, DocumentTask, TaskStatus};
use serde::{Deserialize, Serialize};
use sled::{
    Db, Transactional, Tree,
    transaction::{TransactionError, TransactionResult},
};
use tokio::sync::RwLock;

/// 缓存层方法（内存缓存/过滤器匹配/通用缓存/失效清理）。
mod cache;
/// 维护与备份族方法（过期清理/压缩/后台维护/备份恢复）。
mod maintenance;

/// 存储键前缀
const TASK_PREFIX: &str = "task:";
const INDEX_PREFIX: &str = "index:";
const CACHE_PREFIX: &str = "cache:";
const METADATA_PREFIX: &str = "meta:";

/// 索引类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IndexType {
    ByStatus(TaskStatus),
    ByFormat(DocumentFormat),
    ByCreatedTime(u64), // Unix timestamp
    ByUpdatedTime(u64),
}

/// 查询过滤器
#[derive(Debug, Clone)]
pub struct QueryFilter {
    pub status: Option<TaskStatus>,
    pub format: Option<DocumentFormat>,
    pub created_after: Option<SystemTime>,
    pub created_before: Option<SystemTime>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl Default for QueryFilter {
    fn default() -> Self {
        Self {
            status: None,
            format: None,
            created_after: None,
            created_before: None,
            limit: Some(100),
            offset: None,
        }
    }
}

/// 存储配置
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub cache_ttl: std::time::Duration,
    pub max_cache_size: usize,
    pub cleanup_interval: std::time::Duration,
    pub retention_period: std::time::Duration,
    pub batch_size: usize,
    pub enable_compression: bool,
    pub sync_interval: std::time::Duration,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            cache_ttl: std::time::Duration::from_secs(3600), // 1小时
            max_cache_size: 10000,
            cleanup_interval: std::time::Duration::from_secs(3600), // 1小时
            retention_period: std::time::Duration::from_secs(30 * 24 * 3600), // 30天
            batch_size: 100,
            enable_compression: true,
            sync_interval: std::time::Duration::from_secs(60), // 1分钟
        }
    }
}

/// 存储统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    pub total_tasks: usize,
    pub total_size_bytes: u64,
    pub index_count: usize,
    pub cache_hit_rate: f64,
    pub cache_size: usize,
    pub last_cleanup: Option<SystemTime>,
    pub last_sync: Option<SystemTime>,
    pub transaction_count: u64,
    pub failed_transactions: u64,
    pub average_query_time_ms: f64,
}

/// 事务操作类型
#[derive(Debug, Clone)]
pub enum TransactionOp {
    Insert { key: Vec<u8>, value: Vec<u8> },
    Update { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    IndexUpdate { index_key: Vec<u8>, task_id: String },
    IndexDelete { index_key: Vec<u8> },
}

/// 缓存项
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheItem<T> {
    data: T,
    created_at: SystemTime,
    expires_at: Option<SystemTime>,
    access_count: u64,
}

/// 数据库存储服务
#[derive(Debug)]
pub struct StorageService {
    db: Arc<Db>,
    tasks_tree: Tree,
    index_tree: Tree,
    cache_tree: Tree,
    metadata_tree: Tree,

    // 配置
    config: StorageConfig,

    // 内存缓存
    memory_cache: Arc<RwLock<HashMap<String, CacheItem<DocumentTask>>>>,

    // 统计信息
    stats: Arc<RwLock<StorageStats>>,

    // 事务计数器
    transaction_counter: std::sync::atomic::AtomicU64,
    failed_transaction_counter: std::sync::atomic::AtomicU64,
}

impl StorageService {
    /// 创建新的存储服务
    pub fn new(db: Arc<Db>) -> Result<Self, AppError> {
        Self::with_config(db, StorageConfig::default())
    }

    /// 使用自定义配置创建存储服务
    pub fn with_config(db: Arc<Db>, config: StorageConfig) -> Result<Self, AppError> {
        let tasks_tree = db
            .open_tree("tasks")
            .map_err(|e| AppError::Database(format!("打开任务树失败: {e}")))?;

        let index_tree = db
            .open_tree("indexes")
            .map_err(|e| AppError::Database(format!("打开索引树失败: {e}")))?;

        let cache_tree = db
            .open_tree("cache")
            .map_err(|e| AppError::Database(format!("打开缓存树失败: {e}")))?;

        let metadata_tree = db
            .open_tree("metadata")
            .map_err(|e| AppError::Database(format!("打开元数据树失败: {e}")))?;

        let stats = StorageStats {
            total_tasks: 0,
            total_size_bytes: 0,
            index_count: 0,
            cache_hit_rate: 0.0,
            cache_size: 0,
            last_cleanup: None,
            last_sync: None,
            transaction_count: 0,
            failed_transactions: 0,
            average_query_time_ms: 0.0,
        };

        Ok(Self {
            db,
            tasks_tree,
            index_tree,
            cache_tree,
            metadata_tree,
            config,
            memory_cache: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(stats)),
            transaction_counter: std::sync::atomic::AtomicU64::new(0),
            failed_transaction_counter: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// 保存任务
    pub async fn save_task(&self, task: &DocumentTask) -> Result<(), AppError> {
        let start_time = std::time::Instant::now();

        // 执行事务
        let result = self
            .execute_transaction(|tx_ops| {
                let task_key = format!("{}{}", TASK_PREFIX, task.id);

                // 序列化任务数据
                let task_data = serde_json::to_vec(task)
                    .map_err(|e| AppError::Database(format!("序列化任务失败: {e}")))?;

                // 添加主要操作
                tx_ops.push(TransactionOp::Insert {
                    key: task_key.into_bytes(),
                    value: task_data,
                });

                // 添加索引操作
                self.add_index_operations(task, tx_ops)?;

                Ok(())
            })
            .await;

        match result {
            Ok(()) => {
                // 更新内存缓存
                self.update_memory_cache(&task.id, task.clone()).await;

                // 清除相关缓存
                self.invalidate_cache_for_task(&task.id).await?;

                // 更新统计信息
                self.update_query_stats(start_time.elapsed()).await;

                log::debug!("Task saved: {}", task.id);
                Ok(())
            }
            Err(e) => {
                self.failed_transaction_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Err(e)
            }
        }
    }

    /// 执行事务
    async fn execute_transaction<F>(&self, mut operation: F) -> Result<(), AppError>
    where
        F: FnMut(&mut Vec<TransactionOp>) -> Result<(), AppError>,
    {
        let mut tx_ops = Vec::new();
        operation(&mut tx_ops)?;

        // 执行事务
        let result: TransactionResult<(), ()> =
            (&self.tasks_tree, &self.index_tree).transaction(|(tasks_tree, index_tree)| {
                for op in &tx_ops {
                    match op {
                        TransactionOp::Insert { key, value } => {
                            tasks_tree.insert(key.as_slice(), value.as_slice())?;
                        }
                        TransactionOp::Update { key, value } => {
                            tasks_tree.insert(key.as_slice(), value.as_slice())?;
                        }
                        TransactionOp::Delete { key } => {
                            tasks_tree.remove(key.as_slice())?;
                        }
                        TransactionOp::IndexUpdate { index_key, task_id } => {
                            index_tree.insert(index_key.as_slice(), task_id.as_bytes())?;
                        }
                        TransactionOp::IndexDelete { index_key } => {
                            index_tree.remove(index_key.as_slice())?;
                        }
                    }
                }
                Ok(())
            });

        match result {
            Ok(()) => {
                self.transaction_counter
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
            Err(TransactionError::Abort(e)) => Err(AppError::Database(format!("事务中止: {e:?}"))),
            Err(TransactionError::Storage(e)) => Err(AppError::Database(format!("存储错误: {e}"))),
        }
    }

    /// 获取任务
    pub async fn get_task(&self, task_id: &str) -> Result<Option<DocumentTask>, AppError> {
        let start_time = std::time::Instant::now();

        // 先检查内存缓存
        if let Some(cached_task) = self.get_from_memory_cache(task_id).await {
            self.update_query_stats(start_time.elapsed()).await;
            return Ok(Some(cached_task));
        }

        // 检查持久化缓存
        if let Some(cached_task) = self
            .get_from_cache::<DocumentTask>(&format!("task:{task_id}"))
            .await?
        {
            // 更新内存缓存
            self.update_memory_cache(task_id, cached_task.clone()).await;
            self.update_query_stats(start_time.elapsed()).await;
            return Ok(Some(cached_task));
        }

        let task_key = format!("{TASK_PREFIX}{task_id}");

        match self.tasks_tree.get(&task_key) {
            Ok(Some(data)) => {
                let task: DocumentTask = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Database(format!("反序列化任务失败: {e}")))?;

                // 更新缓存
                self.update_memory_cache(task_id, task.clone()).await;
                self.set_cache(&format!("task:{task_id}"), &task, None)
                    .await?;

                self.update_query_stats(start_time.elapsed()).await;
                Ok(Some(task))
            }
            Ok(None) => {
                self.update_query_stats(start_time.elapsed()).await;
                Ok(None)
            }
            Err(e) => Err(AppError::Database(format!("查询任务失败: {e}"))),
        }
    }

    /// 删除任务
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, AppError> {
        let start_time = std::time::Instant::now();

        // 获取任务以便清理索引
        let task = self.get_task(task_id).await?;

        if let Some(task) = task {
            // 执行删除事务
            let result = self
                .execute_transaction(|tx_ops| {
                    let task_key = format!("{TASK_PREFIX}{task_id}");

                    // 添加删除操作
                    tx_ops.push(TransactionOp::Delete {
                        key: task_key.into_bytes(),
                    });

                    // 添加索引删除操作
                    self.add_index_delete_operations(&task, tx_ops)?;

                    Ok(())
                })
                .await;

            match result {
                Ok(()) => {
                    // 清除缓存
                    self.remove_from_memory_cache(task_id).await;
                    self.invalidate_cache_for_task(task_id).await?;

                    self.update_query_stats(start_time.elapsed()).await;
                    log::info!("Task deleted: {task_id}");
                    Ok(true)
                }
                Err(e) => {
                    self.failed_transaction_counter
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Err(e)
                }
            }
        } else {
            Ok(false)
        }
    }

    /// 批量保存任务
    pub async fn save_tasks_batch(&self, tasks: &[DocumentTask]) -> Result<usize, AppError> {
        let start_time = std::time::Instant::now();
        let mut saved_count = 0;

        // 分批处理
        for chunk in tasks.chunks(self.config.batch_size) {
            let result = self
                .execute_transaction(|tx_ops| {
                    for task in chunk {
                        let task_key = format!("{}{}", TASK_PREFIX, task.id);

                        // 序列化任务数据
                        let task_data = serde_json::to_vec(task)
                            .map_err(|e| AppError::Database(format!("序列化任务失败: {e}")))?;

                        tx_ops.push(TransactionOp::Insert {
                            key: task_key.into_bytes(),
                            value: task_data,
                        });

                        // 添加索引操作
                        self.add_index_operations(task, tx_ops)?;
                    }
                    Ok(())
                })
                .await;

            match result {
                Ok(()) => {
                    saved_count += chunk.len();

                    // 更新内存缓存
                    for task in chunk {
                        self.update_memory_cache(&task.id, task.clone()).await;
                    }
                }
                Err(e) => {
                    log::error!("Batch save failed: {e}");
                    return Err(e);
                }
            }
        }

        self.update_query_stats(start_time.elapsed()).await;
        log::info!("Batch saving completed: {saved_count} tasks");
        Ok(saved_count)
    }

    /// 添加索引操作到事务
    fn add_index_operations(
        &self,
        task: &DocumentTask,
        tx_ops: &mut Vec<TransactionOp>,
    ) -> Result<(), AppError> {
        let task_id = &task.id;

        // 仅基于 task_id 的索引键
        let task_index_key = format!("{INDEX_PREFIX}task:{task_id}");
        tx_ops.push(TransactionOp::IndexUpdate {
            index_key: task_index_key.into_bytes(),
            task_id: task_id.clone(),
        });

        Ok(())
    }

    /// 添加索引删除操作到事务
    fn add_index_delete_operations(
        &self,
        task: &DocumentTask,
        tx_ops: &mut Vec<TransactionOp>,
    ) -> Result<(), AppError> {
        let task_id = &task.id;

        // 删除仅基于 task_id 的索引键
        let task_index_key = format!("{INDEX_PREFIX}task:{task_id}");
        tx_ops.push(TransactionOp::IndexDelete {
            index_key: task_index_key.into_bytes(),
        });

        Ok(())
    }

    /// 更新查询统计信息
    async fn update_query_stats(&self, query_time: std::time::Duration) {
        let mut stats = self.stats.write().await;

        // 更新平均查询时间
        let query_time_ms = query_time.as_millis() as f64;
        if stats.average_query_time_ms == 0.0 {
            stats.average_query_time_ms = query_time_ms;
        } else {
            // 简单的移动平均
            stats.average_query_time_ms =
                (stats.average_query_time_ms * 0.9) + (query_time_ms * 0.1);
        }
    }

    /// 查询任务
    pub async fn query_tasks(&self, filter: &QueryFilter) -> Result<Vec<DocumentTask>, AppError> {
        let cache_key = format!("query:{}", self.filter_to_cache_key(filter));

        // 检查缓存
        if let Some(cached_result) = self.get_from_cache::<Vec<DocumentTask>>(&cache_key).await? {
            return Ok(cached_result);
        }

        let mut results = Vec::new();
        let mut count = 0;
        let offset = filter.offset.unwrap_or(0);
        let limit = filter.limit.unwrap_or(100);

        // 遍历所有任务
        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {e}")))?;

            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {e}")))?;

            // 应用过滤器
            if self.task_matches_filter(&task, filter) {
                if count >= offset {
                    results.push(task);
                    if results.len() >= limit {
                        break;
                    }
                }
                count += 1;
            }
        }

        // 缓存结果
        self.set_cache(
            &cache_key,
            &results,
            Some(std::time::Duration::from_secs(300)),
        )
        .await?;

        Ok(results)
    }

    /// 获取存储统计信息
    pub async fn get_stats(&self) -> Result<StorageStats, AppError> {
        let cache_key = "storage_stats";

        // 检查缓存
        if let Some(cached_stats) = self.get_from_cache::<StorageStats>(cache_key).await? {
            return Ok(cached_stats);
        }

        let mut total_tasks = 0;
        let mut total_size_bytes = 0;

        // 统计任务数量和大小
        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {e}")))?;
            total_tasks += 1;
            total_size_bytes += data.len() as u64;
        }

        // 统计索引数量
        let index_count = self.index_tree.len();

        // 获取内存缓存大小
        let cache_size = {
            let cache = self.memory_cache.read().await;
            cache.len()
        };

        // 计算缓存命中率（简化版本）
        let cache_hit_rate = 0.85; // 占位值，实际应该基于访问统计

        // 获取事务统计
        let transaction_count = self
            .transaction_counter
            .load(std::sync::atomic::Ordering::Relaxed);
        let failed_transactions = self
            .failed_transaction_counter
            .load(std::sync::atomic::Ordering::Relaxed);

        // 获取平均查询时间
        let average_query_time_ms = {
            let stats = self.stats.read().await;
            stats.average_query_time_ms
        };

        let stats = StorageStats {
            total_tasks,
            total_size_bytes,
            index_count,
            cache_hit_rate,
            cache_size,
            last_cleanup: self.get_last_cleanup_time().await?,
            last_sync: {
                let stats = self.stats.read().await;
                stats.last_sync
            },
            transaction_count,
            failed_transactions,
            average_query_time_ms,
        };

        // 缓存统计信息
        self.set_cache(cache_key, &stats, Some(std::time::Duration::from_secs(60)))
            .await?;

        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CreateTaskParams, SourceType};
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_storage_service_basic() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let storage = StorageService::new(db).unwrap();

        // 创建测试任务
        let task_id = uuid::Uuid::new_v4().to_string();
        let mut task = DocumentTask::new(CreateTaskParams {
            id: task_id.clone(),
            source_type: SourceType::Upload,
            source: Some("/test/path".to_string()),
            original_filename: Some("path.pdf".to_string()),
            document_format: Some(DocumentFormat::PDF),
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });
        task.parser_engine = Some(crate::models::ParserEngine::MinerU);
        task.file_size = Some(1024);
        task.mime_type = Some("application/pdf".to_string());

        // 保存任务
        storage.save_task(&task).await.unwrap();

        // 获取任务
        let retrieved = storage.get_task(&task_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, task_id);

        // 删除任务
        let deleted = storage.delete_task(&task_id).await.unwrap();
        assert!(deleted);

        // 确认删除
        let not_found = storage.get_task(&task_id).await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_transaction_handling() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let storage = StorageService::new(db).unwrap();

        // 创建多个任务进行批量保存
        let mut tasks = Vec::new();
        for i in 0..5 {
            let mut task = DocumentTask::new(CreateTaskParams {
                id: format!("batch_task_{i}"),
                source_type: SourceType::Upload,
                source: Some(format!("/test/path_{i}")),
                original_filename: Some(format!("path_{i}.pdf")),
                document_format: Some(DocumentFormat::PDF),
                backend: Some("pipeline".to_string()),
                expires_in_hours: Some(24),
                max_retries: Some(3),
            });
            task.parser_engine = Some(crate::models::ParserEngine::MinerU);
            task.file_size = Some(1024);
            task.mime_type = Some("application/pdf".to_string());
            tasks.push(task);
        }

        // 批量保存
        let saved_count = storage.save_tasks_batch(&tasks).await.unwrap();
        assert_eq!(saved_count, 5);

        // 验证所有任务都已保存
        for i in 0..5 {
            let task_id = format!("batch_task_{i}");
            let retrieved = storage.get_task(&task_id).await.unwrap();
            assert!(retrieved.is_some());
        }
    }

    #[tokio::test]
    async fn test_memory_cache() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());

        let config = StorageConfig {
            max_cache_size: 2, // 限制缓存大小
            ..Default::default()
        };

        let storage = StorageService::with_config(db, config).unwrap();

        // 创建测试任务
        let task1_id = uuid::Uuid::new_v4().to_string();
        let mut task1 = DocumentTask::new(CreateTaskParams {
            id: task1_id.clone(),
            source_type: SourceType::Upload,
            source: Some("/test/path1".to_string()),
            original_filename: Some("path1.pdf".to_string()),
            document_format: Some(DocumentFormat::PDF),
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });
        task1.parser_engine = Some(crate::models::ParserEngine::MinerU);
        task1.file_size = Some(1024);
        task1.mime_type = Some("application/pdf".to_string());

        let task2_id = uuid::Uuid::new_v4().to_string();
        let mut task2 = DocumentTask::new(CreateTaskParams {
            id: task2_id.clone(),
            source_type: SourceType::Upload,
            source: Some("/test/path2".to_string()),
            original_filename: Some("path2.pdf".to_string()),
            document_format: Some(DocumentFormat::PDF),
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });
        task2.parser_engine = Some(crate::models::ParserEngine::MinerU);
        task2.file_size = Some(1024);
        task2.mime_type = Some("application/pdf".to_string());

        // 保存任务（会自动缓存）
        storage.save_task(&task1).await.unwrap();
        storage.save_task(&task2).await.unwrap();

        // 第一次获取应该从缓存中获取
        let retrieved1 = storage.get_task(&task1_id).await.unwrap();
        assert!(retrieved1.is_some());

        // 检查缓存状态
        let cache_size = {
            let cache = storage.memory_cache.read().await;
            cache.len()
        };
        assert!(cache_size <= 2); // 不应该超过最大缓存大小
    }

    #[tokio::test]
    async fn test_cleanup_expired_data() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());

        let config = StorageConfig {
            retention_period: std::time::Duration::from_secs(1), // 1秒过期
            ..Default::default()
        };

        let storage = StorageService::with_config(db, config).unwrap();

        // 创建已完成的任务
        let task_id = uuid::Uuid::new_v4().to_string();
        let mut task = DocumentTask::new(CreateTaskParams {
            id: task_id.clone(),
            source_type: SourceType::Upload,
            source: Some("/test/path".to_string()),
            original_filename: Some("path.pdf".to_string()),
            document_format: Some(DocumentFormat::PDF),
            backend: Some("pipeline".to_string()),
            expires_in_hours: Some(24),
            max_retries: Some(3),
        });
        task.parser_engine = Some(crate::models::ParserEngine::MinerU);
        task.file_size = Some(1024);
        task.mime_type = Some("application/pdf".to_string());

        // 设置为已完成状态
        let _ = task.update_status(TaskStatus::new_completed(std::time::Duration::from_secs(
            60,
        )));

        storage.save_task(&task).await.unwrap();

        // 等待过期
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // 执行清理
        let cleaned_count = storage.cleanup_expired_data().await.unwrap();
        assert!(cleaned_count > 0);

        // 验证任务已被删除
        let retrieved = storage.get_task(&task_id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let storage = StorageService::new(db).unwrap();

        // 创建一些测试任务
        for i in 0..3 {
            let task_id = uuid::Uuid::new_v4().to_string();
            let mut task = DocumentTask::new(CreateTaskParams {
                id: task_id,
                source_type: SourceType::Upload,
                source: Some(format!("/test/path_{i}")),
                original_filename: Some(format!("path_{i}.pdf")),
                document_format: Some(DocumentFormat::PDF),
                backend: Some("pipeline".to_string()),
                expires_in_hours: Some(24),
                max_retries: Some(3),
            });
            task.parser_engine = Some(crate::models::ParserEngine::MinerU);
            task.file_size = Some(1024);
            task.mime_type = Some("application/pdf".to_string());

            storage.save_task(&task).await.unwrap();
        }

        // 获取统计信息
        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_tasks, 3);
        assert!(stats.total_size_bytes > 0);
        assert!(stats.transaction_count > 0);
    }
}
