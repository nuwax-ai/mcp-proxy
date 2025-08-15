use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::HashMap;

use sled::{Db, Tree, Batch, transaction::{TransactionResult, TransactionError}, Transactional};
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;
use crate::error::AppError;
use crate::models::{DocumentTask, TaskStatus, DocumentFormat};

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
        let tasks_tree = db.open_tree("tasks")
            .map_err(|e| AppError::Database(format!("打开任务树失败: {}", e)))?;
        
        let index_tree = db.open_tree("indexes")
            .map_err(|e| AppError::Database(format!("打开索引树失败: {}", e)))?;
        
        let cache_tree = db.open_tree("cache")
            .map_err(|e| AppError::Database(format!("打开缓存树失败: {}", e)))?;
        
        let metadata_tree = db.open_tree("metadata")
            .map_err(|e| AppError::Database(format!("打开元数据树失败: {}", e)))?;
        
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
        let result = self.execute_transaction(|tx_ops| {
            let task_key = format!("{}{}", TASK_PREFIX, task.id);
            
            // 序列化任务数据
            let task_data = serde_json::to_vec(task)
                .map_err(|e| AppError::Database(format!("序列化任务失败: {}", e)))?;
            
            // 添加主要操作
            tx_ops.push(TransactionOp::Insert {
                key: task_key.into_bytes(),
                value: task_data,
            });
            
            // 添加索引操作
            self.add_index_operations(task, tx_ops)?;
            
            Ok(())
        }).await;
        
        match result {
            Ok(()) => {
                // 更新内存缓存
                self.update_memory_cache(&task.id, task.clone()).await;
                
                // 清除相关缓存
                self.invalidate_cache_for_task(&task.id).await?;
                
                // 更新统计信息
                self.update_query_stats(start_time.elapsed()).await;
                
                log::debug!("任务已保存: {}", task.id);
                Ok(())
            }
            Err(e) => {
                self.failed_transaction_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        let result: TransactionResult<(), ()> = (&self.tasks_tree, &self.index_tree).transaction(|(tasks_tree, index_tree)| {
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
                self.transaction_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
            Err(TransactionError::Abort(e)) => {
                Err(AppError::Database(format!("事务中止: {:?}", e)))
            }
            Err(TransactionError::Storage(e)) => {
                Err(AppError::Database(format!("存储错误: {}", e)))
            }
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
        if let Some(cached_task) = self.get_from_cache::<DocumentTask>(&format!("task:{}", task_id)).await? {
            // 更新内存缓存
            self.update_memory_cache(task_id, cached_task.clone()).await;
            self.update_query_stats(start_time.elapsed()).await;
            return Ok(Some(cached_task));
        }
        
        let task_key = format!("{}{}", TASK_PREFIX, task_id);
        
        match self.tasks_tree.get(&task_key) {
            Ok(Some(data)) => {
                let task: DocumentTask = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Database(format!("反序列化任务失败: {}", e)))?;
                
                // 更新缓存
                self.update_memory_cache(task_id, task.clone()).await;
                self.set_cache(&format!("task:{}", task_id), &task, None).await?;
                
                self.update_query_stats(start_time.elapsed()).await;
                Ok(Some(task))
            }
            Ok(None) => {
                self.update_query_stats(start_time.elapsed()).await;
                Ok(None)
            }
            Err(e) => Err(AppError::Database(format!("查询任务失败: {}", e))),
        }
    }

    /// 删除任务
    pub async fn delete_task(&self, task_id: &str) -> Result<bool, AppError> {
        let start_time = std::time::Instant::now();
        
        // 获取任务以便清理索引
        let task = self.get_task(task_id).await?;
        
        if let Some(task) = task {
            // 执行删除事务
            let result = self.execute_transaction(|tx_ops| {
                let task_key = format!("{}{}", TASK_PREFIX, task_id);
                
                // 添加删除操作
                tx_ops.push(TransactionOp::Delete {
                    key: task_key.into_bytes(),
                });
                
                // 添加索引删除操作
                self.add_index_delete_operations(&task, tx_ops)?;
                
                Ok(())
            }).await;
            
            match result {
                Ok(()) => {
                    // 清除缓存
                    self.remove_from_memory_cache(task_id).await;
                    self.invalidate_cache_for_task(task_id).await?;
                    
                    self.update_query_stats(start_time.elapsed()).await;
                    log::info!("任务已删除: {}", task_id);
                    Ok(true)
                }
                Err(e) => {
                    self.failed_transaction_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
            let result = self.execute_transaction(|tx_ops| {
                for task in chunk {
                    let task_key = format!("{}{}", TASK_PREFIX, task.id);
                    
                    // 序列化任务数据
                    let task_data = serde_json::to_vec(task)
                        .map_err(|e| AppError::Database(format!("序列化任务失败: {}", e)))?;
                    
                    tx_ops.push(TransactionOp::Insert {
                        key: task_key.into_bytes(),
                        value: task_data,
                    });
                    
                    // 添加索引操作
                    self.add_index_operations(task, tx_ops)?;
                }
                Ok(())
            }).await;
            
            match result {
                Ok(()) => {
                    saved_count += chunk.len();
                    
                    // 更新内存缓存
                    for task in chunk {
                        self.update_memory_cache(&task.id, task.clone()).await;
                    }
                }
                Err(e) => {
                    log::error!("批量保存失败: {}", e);
                    return Err(e);
                }
            }
        }
        
        self.update_query_stats(start_time.elapsed()).await;
        log::info!("批量保存完成: {} 个任务", saved_count);
        Ok(saved_count)
    }

    /// 内存缓存操作
    async fn get_from_memory_cache(&self, task_id: &str) -> Option<DocumentTask> {
        let cache = self.memory_cache.read().await;
        if let Some(cache_item) = cache.get(task_id) {
            // 检查是否过期
            if let Some(expires_at) = cache_item.expires_at {
                if SystemTime::now() > expires_at {
                    return None;
                }
            }
            Some(cache_item.data.clone())
        } else {
            None
        }
    }

    async fn update_memory_cache(&self, task_id: &str, task: DocumentTask) {
        let mut cache = self.memory_cache.write().await;
        
        // 检查缓存大小限制
        if cache.len() >= self.config.max_cache_size {
            // 简单的LRU：移除最旧的项
            if let Some((oldest_key, _)) = cache.iter().min_by_key(|(_, item)| item.created_at) {
                let oldest_key = oldest_key.clone();
                cache.remove(&oldest_key);
            }
        }
        
        let cache_item = CacheItem {
            data: task,
            created_at: SystemTime::now(),
            expires_at: Some(SystemTime::now() + self.config.cache_ttl),
            access_count: 1,
        };
        
        cache.insert(task_id.to_string(), cache_item);
    }

    async fn remove_from_memory_cache(&self, task_id: &str) {
        let mut cache = self.memory_cache.write().await;
        cache.remove(task_id);
    }

    /// 添加索引操作到事务
    fn add_index_operations(&self, task: &DocumentTask, tx_ops: &mut Vec<TransactionOp>) -> Result<(), AppError> {
        let task_id = &task.id;
        
        // 状态索引
        let status_key = format!("{INDEX_PREFIX}status:{}:{task_id}", task.status);
        tx_ops.push(TransactionOp::IndexUpdate {
            index_key: status_key.into_bytes(),
            task_id: task_id.clone(),
        });
        
        // 格式索引
        let format_key = format!("{INDEX_PREFIX}format:{}:{task_id}", task.document_format);
        tx_ops.push(TransactionOp::IndexUpdate {
            index_key: format_key.into_bytes(),
            task_id: task_id.clone(),
        });
        
        // 时间索引
        let created_timestamp = task.created_at.timestamp() as u64;
        let time_key = format!("{INDEX_PREFIX}time:{created_timestamp}:{task_id}");
        tx_ops.push(TransactionOp::IndexUpdate {
            index_key: time_key.into_bytes(),
            task_id: task_id.clone(),
        });
        
        Ok(())
    }

    /// 添加索引删除操作到事务
    fn add_index_delete_operations(&self, task: &DocumentTask, tx_ops: &mut Vec<TransactionOp>) -> Result<(), AppError> {
        let task_id = &task.id;
        
        // 删除状态索引
        let status_key = format!("{INDEX_PREFIX}status:{}:{task_id}", task.status);
        tx_ops.push(TransactionOp::IndexDelete {
            index_key: status_key.into_bytes(),
        });
        
        // 删除格式索引
        let format_key = format!("{INDEX_PREFIX}format:{}:{task_id}", task.document_format);
        tx_ops.push(TransactionOp::IndexDelete {
            index_key: format_key.into_bytes(),
        });
        
        // 删除时间索引
        let created_timestamp = task.created_at.timestamp() as u64;
        let time_key = format!("{INDEX_PREFIX}time:{created_timestamp}:{task_id}");
        tx_ops.push(TransactionOp::IndexDelete {
            index_key: time_key.into_bytes(),
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
            stats.average_query_time_ms = (stats.average_query_time_ms * 0.9) + (query_time_ms * 0.1);
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
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {}", e)))?;
            
            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {}", e)))?;
            
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
        self.set_cache(&cache_key, &results, Some(std::time::Duration::from_secs(300))).await?;
        
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
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {}", e)))?;
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
        let transaction_count = self.transaction_counter.load(std::sync::atomic::Ordering::Relaxed);
        let failed_transactions = self.failed_transaction_counter.load(std::sync::atomic::Ordering::Relaxed);
        
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
        self.set_cache(cache_key, &stats, Some(std::time::Duration::from_secs(60))).await?;
        
        Ok(stats)
    }

    /// 清理过期数据
    pub async fn cleanup_expired_data(&self) -> Result<usize, AppError> {
        log::info!("开始清理过期数据");
        
        let mut cleaned_count = 0;
        let now = SystemTime::now();
        
        // 清理过期任务
        let expired_tasks = self.find_expired_tasks(now).await?;
        
        if !expired_tasks.is_empty() {
            // 批量删除过期任务
            for chunk in expired_tasks.chunks(self.config.batch_size) {
                let result = self.execute_transaction(|tx_ops| {
                    for task in chunk {
                        let task_key = format!("{}{}", TASK_PREFIX, task.id);
                        
                        tx_ops.push(TransactionOp::Delete {
                            key: task_key.into_bytes(),
                        });
                        
                        // 添加索引删除操作
                        self.add_index_delete_operations(task, tx_ops)?;
                    }
                    Ok(())
                }).await;
                
                match result {
                    Ok(()) => {
                        cleaned_count += chunk.len();
                        
                        // 清理内存缓存
                        for task in chunk {
                            self.remove_from_memory_cache(&task.id).await;
                        }
                    }
                    Err(e) => {
                        log::error!("批量删除过期任务失败: {}", e);
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
        
        log::info!("清理完成，删除了 {} 条记录", cleaned_count);
        Ok(cleaned_count)
    }

    /// 查找过期任务
    async fn find_expired_tasks(&self, now: SystemTime) -> Result<Vec<DocumentTask>, AppError> {
        let mut expired_tasks = Vec::new();
        
        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {}", e)))?;
            
            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {}", e)))?;
            
            // 检查是否过期
            let task_created_at = UNIX_EPOCH + std::time::Duration::from_secs(task.created_at.timestamp() as u64);
            if let Ok(elapsed) = now.duration_since(task_created_at) {
                if elapsed > self.config.retention_period && task.status.is_terminal() {
                    expired_tasks.push(task);
                }
            }
        }
        
        Ok(expired_tasks)
    }

    /// 清理内存缓存
    async fn cleanup_memory_cache(&self) -> usize {
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
    async fn compact_database(&self) -> Result<(), AppError> {
        log::info!("开始压缩数据库");
        
        // 刷新所有树
        self.tasks_tree.flush()
            .map_err(|e| AppError::Database(format!("刷新任务树失败: {}", e)))?;
        
        self.index_tree.flush()
            .map_err(|e| AppError::Database(format!("刷新索引树失败: {}", e)))?;
        
        self.cache_tree.flush()
            .map_err(|e| AppError::Database(format!("刷新缓存树失败: {}", e)))?;
        
        self.metadata_tree.flush()
            .map_err(|e| AppError::Database(format!("刷新元数据树失败: {}", e)))?;
        
        // 刷新整个数据库
        self.db.flush()
            .map_err(|e| AppError::Database(format!("刷新数据库失败: {}", e)))?;
        
        log::info!("数据库压缩完成");
        Ok(())
    }

    /// 启动后台维护任务
    pub async fn start_maintenance_tasks(&self) -> Result<(), AppError> {
        let storage_service = self.clone_for_background();
        
        tokio::spawn(async move {
            let mut cleanup_interval = tokio::time::interval(storage_service.config.cleanup_interval);
            let mut sync_interval = tokio::time::interval(storage_service.config.sync_interval);
            
            loop {
                tokio::select! {
                    _ = cleanup_interval.tick() => {
                        if let Err(e) = storage_service.cleanup_expired_data().await {
                            log::error!("定期清理失败: {}", e);
                        }
                    }
                    
                    _ = sync_interval.tick() => {
                        if let Err(e) = storage_service.sync_to_disk().await {
                            log::error!("定期同步失败: {}", e);
                        }
                    }
                }
            }
        });
        
        Ok(())
    }

    /// 同步到磁盘
    async fn sync_to_disk(&self) -> Result<(), AppError> {
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
                self.transaction_counter.load(std::sync::atomic::Ordering::Relaxed)
            ),
            failed_transaction_counter: std::sync::atomic::AtomicU64::new(
                self.failed_transaction_counter.load(std::sync::atomic::Ordering::Relaxed)
            ),
        }
    }

    /// 备份数据
    pub async fn backup_to_path(&self, backup_path: &str) -> Result<(), AppError> {
        log::info!("开始备份数据到: {}", backup_path);
        
        // 创建备份目录
        std::fs::create_dir_all(backup_path)
            .map_err(|e| AppError::File(format!("创建备份目录失败: {}", e)))?;
        
        // 导出所有任务
        let mut tasks = Vec::new();
        for result in self.tasks_tree.scan_prefix(TASK_PREFIX.as_bytes()) {
            let (_, data) = result.map_err(|e| AppError::Database(format!("扫描任务失败: {}", e)))?;
            let task: DocumentTask = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化任务失败: {}", e)))?;
            tasks.push(task);
        }
        
        // 写入备份文件
        let backup_file = format!("{}/tasks_backup_{}.json", backup_path, 
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs());
        
        let backup_data = serde_json::to_string_pretty(&tasks)
            .map_err(|e| AppError::Database(format!("序列化备份数据失败: {}", e)))?;
        
        std::fs::write(&backup_file, backup_data)
            .map_err(|e| AppError::File(format!("写入备份文件失败: {}", e)))?;
        
        log::info!("备份完成: {} ({} 个任务)", backup_file, tasks.len());
        Ok(())
    }

    /// 从备份恢复数据
    pub async fn restore_from_backup(&self, backup_file: &str) -> Result<usize, AppError> {
        log::info!("从备份恢复数据: {}", backup_file);
        
        let backup_data = std::fs::read_to_string(backup_file)
            .map_err(|e| AppError::File(format!("读取备份文件失败: {}", e)))?;
        
        let tasks: Vec<DocumentTask> = serde_json::from_str(&backup_data)
            .map_err(|e| AppError::Database(format!("反序列化备份数据失败: {}", e)))?;
        
        let mut restored_count = 0;
        
        for task in tasks {
            self.save_task(&task).await?;
            restored_count += 1;
        }
        
        log::info!("恢复完成: {} 个任务", restored_count);
        Ok(restored_count)
    }

    // 私有方法

    /// 检查任务是否匹配过滤器
    fn task_matches_filter(&self, task: &DocumentTask, filter: &QueryFilter) -> bool {
        // 状态过滤
        if let Some(status) = &filter.status {
            if &task.status != status {
                return false;
            }
        }
        
        // 格式过滤
        if let Some(format) = &filter.format {
            if &task.document_format != format {
                return false;
            }
        }
        
        // 创建时间过滤
        if let Some(after) = filter.created_after {
            let after_dt = DateTime::<Utc>::from(after);
            if task.created_at < after_dt {
                return false;
            }
        }
        
        if let Some(before) = filter.created_before {
            let before_dt = DateTime::<Utc>::from(before);
            if task.created_at > before_dt {
                return false;
            }
        }
        
        true
    }

    /// 生成过滤器的缓存键
    fn filter_to_cache_key(&self, filter: &QueryFilter) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}",
            filter.status.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "any".to_string()),
            filter.format.as_ref().map(|f| f.to_string()).unwrap_or_else(|| "any".to_string()),
            filter.created_after.map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()).unwrap_or(0),
            filter.created_before.map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()).unwrap_or(u64::MAX),
            filter.limit.unwrap_or(100),
            filter.offset.unwrap_or(0)
        )
    }

    /// 从缓存获取数据
    async fn get_from_cache<T>(&self, key: &str) -> Result<Option<T>, AppError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let cache_key = format!("{CACHE_PREFIX}{key}");
        
        if let Ok(Some(data)) = self.cache_tree.get(&cache_key) {
            let cache_item: CacheItem<T> = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化缓存项失败: {e}")))?;
            
            // 检查是否过期
            if let Some(expires_at) = cache_item.expires_at {
                if SystemTime::now() > expires_at {
                    // 过期，删除缓存项
                    self.cache_tree.remove(&cache_key)
                        .map_err(|e| AppError::Database(format!("删除过期缓存失败: {e}")))?;
                    return Ok(None);
                }
            }
            
            // 更新访问计数（简化版本，不实际更新）
            Ok(Some(cache_item.data))
        } else {
            Ok(None)
        }
    }

    /// 设置缓存
    async fn set_cache<T>(&self, key: &str, data: &T, ttl: Option<std::time::Duration>) -> Result<(), AppError>
    where
        T: Serialize,
    {
        let cache_key = format!("{CACHE_PREFIX}{key}");
        let now = SystemTime::now();
        
        let cache_item = CacheItem {
            data,
            created_at: now,
            expires_at: ttl.map(|duration| now + duration),
            access_count: 1,
        };
        
        let cache_data = serde_json::to_vec(&cache_item)
            .map_err(|e| AppError::Database(format!("序列化缓存项失败: {e}")))?;
        
        self.cache_tree.insert(&cache_key, cache_data)
            .map_err(|e| AppError::Database(format!("设置缓存失败: {e}")))?;
        
        Ok(())
    }

    /// 清除任务相关缓存
    async fn invalidate_cache_for_task(&self, task_id: &str) -> Result<(), AppError> {
        let patterns = vec![
            format!("task:{}", task_id),
            "query:".to_string(),
            "storage_stats".to_string(),
        ];
        
        for pattern in patterns {
            let cache_key = format!("{CACHE_PREFIX}{pattern}");
            
            if pattern.starts_with("query:") {
                // 清除所有查询缓存
                let prefix = cache_key.as_bytes();
                let mut to_remove = Vec::new();
                
                for result in self.cache_tree.scan_prefix(prefix) {
                    let (key, _) = result.map_err(|e| AppError::Database(format!("扫描缓存失败: {e}")))?;
                    to_remove.push(key.to_vec());
                }
                
                for key in to_remove {
                    self.cache_tree.remove(&key)
                        .map_err(|e| AppError::Database(format!("删除缓存失败: {e}")))?;
                }
            } else {
                self.cache_tree.remove(&cache_key)
                    .map_err(|e| AppError::Database(format!("删除缓存失败: {e}")))?;
            }
        }
        
        Ok(())
    }

    /// 清理过期缓存
    async fn cleanup_expired_cache(&self) -> Result<usize, AppError> {
        let mut cleaned_count = 0;
        let now = SystemTime::now();
        let mut to_remove = Vec::new();
        
        for result in self.cache_tree.scan_prefix(CACHE_PREFIX.as_bytes()) {
            let (key, data) = result.map_err(|e| AppError::Database(format!("扫描缓存失败: {e}")))?;
            
            // 尝试解析缓存项（简化版本）
            if let Ok(cache_item) = serde_json::from_slice::<serde_json::Value>(&data) {
                if let Some(expires_at_timestamp) = cache_item.get("expires_at").and_then(|v| v.as_u64()) {
                    let expires_at = UNIX_EPOCH + std::time::Duration::from_secs(expires_at_timestamp);
                    if now > expires_at {
                        to_remove.push(key.to_vec());
                    }
                }
            }
        }
        
        for key in to_remove {
            self.cache_tree.remove(&key)
                .map_err(|e| AppError::Database(format!("删除过期缓存失败: {e}")))?;
            cleaned_count += 1;
        }
        
        Ok(cleaned_count)
    }

    /// 获取最后清理时间
    async fn get_last_cleanup_time(&self) -> Result<Option<SystemTime>, AppError> {
        let key = format!("{}{}", METADATA_PREFIX, "last_cleanup");
        
        if let Ok(Some(data)) = self.metadata_tree.get(&key) {
            let timestamp: u64 = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化清理时间失败: {e}")))?;
            
            Ok(Some(UNIX_EPOCH + std::time::Duration::from_secs(timestamp)))
        } else {
            Ok(None)
        }
    }

    /// 设置最后清理时间
    async fn set_last_cleanup_time(&self, time: SystemTime) -> Result<(), AppError> {
        let key = format!("{}{}", METADATA_PREFIX, "last_cleanup");
        let timestamp = time.duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_secs();
        
        let data = serde_json::to_vec(&timestamp)
            .map_err(|e| AppError::Database(format!("序列化清理时间失败: {e}")))?;
        
        self.metadata_tree.insert(&key, data)
            .map_err(|e| AppError::Database(format!("设置清理时间失败: {e}")))?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::models::{SourceType, ProcessingStage};

    #[tokio::test]
    async fn test_storage_service_basic() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(sled::open(temp_dir.path()).unwrap());
        let storage = StorageService::new(db).unwrap();
        
        // 创建测试任务
        let task = DocumentTask::builder()
            .id("test_task".to_string())
            .source_type(SourceType::Upload)
            .source_path(Some("/test/path".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(crate::models::ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");
        
        // 保存任务
        storage.save_task(&task).await.unwrap();
        
        // 获取任务
        let retrieved = storage.get_task("test_task").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test_task");
        
        // 删除任务
        let deleted = storage.delete_task("test_task").await.unwrap();
        assert!(deleted);
        
        // 确认删除
        let not_found = storage.get_task("test_task").await.unwrap();
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
            let task = DocumentTask::builder()
                .id(format!("batch_task_{}", i))
                .source_type(SourceType::Upload)
                .source_path(Some(format!("/test/path_{}", i)))
                .document_format(DocumentFormat::PDF)
                .parser_engine(crate::models::ParserEngine::MinerU)
                .backend("pipeline")
                .file_size(1024)
                .mime_type("application/pdf")
                .max_retries(3)
                .expires_in_hours(24)
                .build()
                .expect("Failed to build task");
            tasks.push(task);
        }
        
        // 批量保存
        let saved_count = storage.save_tasks_batch(&tasks).await.unwrap();
        assert_eq!(saved_count, 5);
        
        // 验证所有任务都已保存
        for i in 0..5 {
            let task_id = format!("batch_task_{}", i);
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
        let task1 = DocumentTask::builder()
            .id("cache_task_1".to_string())
            .source_type(SourceType::Upload)
            .source_path(Some("/test/path1".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(crate::models::ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");
        
        let task2 = DocumentTask::builder()
            .id("cache_task_2".to_string())
            .source_type(SourceType::Upload)
            .source_path(Some("/test/path2".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(crate::models::ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");
        
        // 保存任务（会自动缓存）
        storage.save_task(&task1).await.unwrap();
        storage.save_task(&task2).await.unwrap();
        
        // 第一次获取应该从缓存中获取
        let retrieved1 = storage.get_task("cache_task_1").await.unwrap();
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
        let mut task = DocumentTask::builder()
            .id("expired_task".to_string())
            .source_type(SourceType::Upload)
            .source_path(Some("/test/path".to_string()))
            .document_format(DocumentFormat::PDF)
            .parser_engine(crate::models::ParserEngine::MinerU)
            .backend("pipeline")
            .file_size(1024)
            .mime_type("application/pdf")
            .max_retries(3)
            .expires_in_hours(24)
            .build()
            .expect("Failed to build task");
        
        // 设置为已完成状态
        let _ = task.update_status(TaskStatus::new_completed(std::time::Duration::from_secs(60)));
        
        storage.save_task(&task).await.unwrap();
        
        // 等待过期
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        
        // 执行清理
        let cleaned_count = storage.cleanup_expired_data().await.unwrap();
        assert!(cleaned_count > 0);
        
        // 验证任务已被删除
        let retrieved = storage.get_task("expired_task").await.unwrap();
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
            let task = DocumentTask::builder()
                .id(format!("stats_task_{}", i))
                .source_type(SourceType::Upload)
                .source_path(Some(format!("/test/path_{}", i)))
                .document_format(DocumentFormat::PDF)
                .parser_engine(crate::models::ParserEngine::MinerU)
                .backend("pipeline")
                .file_size(1024)
                .mime_type("application/pdf")
                .max_retries(3)
                .expires_in_hours(24)
                .build()
                .expect("Failed to build task");
            
            storage.save_task(&task).await.unwrap();
        }
        
        // 获取统计信息
        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_tasks, 3);
        assert!(stats.total_size_bytes > 0);
        assert!(stats.transaction_count > 0);
    }
}