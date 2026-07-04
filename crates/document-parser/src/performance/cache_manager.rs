//! 缓存管理器
//!
//! 提供智能缓存策略、缓存预热和失效管理功能

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Mutex;
use tokio::time::interval;

use super::{CacheConfig, PerformanceOptimizable};
use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::DocumentFormat;

/// 缓存管理器
pub struct CacheManager {
    config: CacheConfig,
    document_cache: Arc<DocumentCache>,
    result_cache: Arc<ResultCache>,
    metadata_cache: Arc<MetadataCache>,
    cache_stats: Arc<CacheStats>,
    eviction_policy: EvictionPolicy,
    preloader: Arc<CachePreloader>,
}

impl CacheManager {
    /// 创建新的缓存管理器
    pub async fn new(_config: &AppConfig) -> Result<Self, AppError> {
        let cache_config = CacheConfig::default(); // 从配置中获取

        let document_cache = Arc::new(DocumentCache::new(cache_config.document_cache_size));
        let result_cache = Arc::new(ResultCache::new(cache_config.result_cache_size));
        let metadata_cache = Arc::new(MetadataCache::new(cache_config.metadata_cache_size));
        let cache_stats = Arc::new(CacheStats::new());
        let eviction_policy = EvictionPolicy::LRU; // 从配置中获取
        let preloader = Arc::new(CachePreloader::new(document_cache.clone()).await?);

        let manager = Self {
            config: cache_config,
            document_cache,
            result_cache,
            metadata_cache,
            cache_stats,
            eviction_policy,
            preloader,
        };

        // 启动后台清理任务
        manager.start_cleanup_task().await;

        Ok(manager)
    }

    /// 缓存文档
    pub async fn cache_document(&self, key: &str, document: Vec<u8>) -> Result<(), AppError> {
        let cache_key = self.generate_cache_key(key);
        let entry = CacheEntry::new(document, self.config.document_ttl);

        self.document_cache.insert(cache_key.clone(), entry).await;
        self.cache_stats.record_document_cache_write().await;

        // 检查是否需要驱逐
        if self.document_cache.should_evict().await {
            self.evict_documents().await?;
        }

        Ok(())
    }

    /// 获取缓存的文档
    pub async fn get_cached_document(&self, key: &str) -> Option<Vec<u8>> {
        let cache_key = self.generate_cache_key(key);

        if let Some(entry) = self.document_cache.get(&cache_key).await {
            if !entry.is_expired() {
                self.cache_stats.record_document_cache_hit().await;
                return Some(entry.data.clone());
            } else {
                // 移除过期条目
                self.document_cache.remove(&cache_key).await;
            }
        }

        self.cache_stats.record_document_cache_miss().await;
        None
    }

    /// 缓存解析结果
    pub async fn cache_result(&self, task_id: &str, result: ParseResult) -> Result<(), AppError> {
        let cache_key = format!("result:{task_id}");
        let entry = CacheEntry::new(result, self.config.result_ttl);

        self.result_cache.insert(cache_key, entry).await;
        self.cache_stats.record_result_cache_write().await;

        if self.result_cache.should_evict().await {
            self.evict_results().await?;
        }

        Ok(())
    }

    /// 获取缓存的解析结果
    pub async fn get_cached_result(&self, task_id: &str) -> Option<ParseResult> {
        let cache_key = format!("result:{task_id}");

        if let Some(entry) = self.result_cache.get(&cache_key).await {
            if !entry.is_expired() {
                self.cache_stats.record_result_cache_hit().await;
                return Some(entry.data.clone());
            } else {
                self.result_cache.remove(&cache_key).await;
            }
        }

        self.cache_stats.record_result_cache_miss().await;
        None
    }

    /// 缓存元数据
    pub async fn cache_metadata(
        &self,
        key: &str,
        metadata: DocumentMetadata,
    ) -> Result<(), AppError> {
        let cache_key = format!("metadata:{key}");
        let entry = CacheEntry::new(metadata, self.config.metadata_ttl);

        self.metadata_cache.insert(cache_key, entry).await;
        self.cache_stats.record_metadata_cache_write().await;

        if self.metadata_cache.should_evict().await {
            self.evict_metadata().await?;
        }

        Ok(())
    }

    /// 获取缓存的元数据
    pub async fn get_cached_metadata(&self, key: &str) -> Option<DocumentMetadata> {
        let cache_key = format!("metadata:{key}");

        if let Some(entry) = self.metadata_cache.get(&cache_key).await {
            if !entry.is_expired() {
                self.cache_stats.record_metadata_cache_hit().await;
                return Some(entry.data.clone());
            } else {
                self.metadata_cache.remove(&cache_key).await;
            }
        }

        self.cache_stats.record_metadata_cache_miss().await;
        None
    }

    /// 预热缓存
    pub async fn warmup_cache(&self, documents: Vec<String>) -> Result<(), AppError> {
        self.preloader.warmup(documents).await
    }

    /// 智能预加载
    pub async fn smart_preload(&self) -> Result<(), AppError> {
        self.preloader.smart_preload().await
    }

    /// 清除所有缓存
    pub async fn clear_all(&self) -> Result<(), AppError> {
        self.document_cache.clear().await;
        self.result_cache.clear().await;
        self.metadata_cache.clear().await;
        self.cache_stats.record_cache_clear().await;

        Ok(())
    }

    /// 清除过期缓存
    pub async fn clear_expired(&self) -> Result<(), AppError> {
        let expired_count = self.document_cache.remove_expired().await
            + self.result_cache.remove_expired().await
            + self.metadata_cache.remove_expired().await;

        self.cache_stats.record_expired_cleanup(expired_count).await;

        Ok(())
    }

    /// 获取缓存统计
    pub async fn get_cache_stats(&self) -> Result<CacheStats, AppError> {
        Ok(self.cache_stats.clone_stats().await)
    }

    /// 获取缓存使用情况
    pub async fn get_cache_usage(&self) -> CacheUsage {
        CacheUsage {
            document_cache: self.document_cache.get_usage().await,
            result_cache: self.result_cache.get_usage().await,
            metadata_cache: self.metadata_cache.get_usage().await,
        }
    }

    /// 调整缓存大小
    pub async fn resize_cache(
        &self,
        cache_type: CacheType,
        new_size: usize,
    ) -> Result<(), AppError> {
        match cache_type {
            CacheType::Document => self.document_cache.resize(new_size).await?,
            CacheType::Result => self.result_cache.resize(new_size).await?,
            CacheType::Metadata => self.metadata_cache.resize(new_size).await?,
        }

        self.cache_stats
            .record_cache_resize(cache_type, new_size)
            .await;

        Ok(())
    }

    // 私有方法

    fn generate_cache_key(&self, key: &str) -> String {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        format!("doc:{:x}", hasher.finish())
    }

    async fn evict_documents(&self) -> Result<(), AppError> {
        match self.eviction_policy {
            EvictionPolicy::LRU => self.document_cache.evict_lru().await,
            EvictionPolicy::LFU => self.document_cache.evict_lfu().await,
            EvictionPolicy::FIFO => self.document_cache.evict_fifo().await,
            EvictionPolicy::Random => self.document_cache.evict_random().await,
        }

        self.cache_stats.record_document_eviction().await;
        Ok(())
    }

    async fn evict_results(&self) -> Result<(), AppError> {
        match self.eviction_policy {
            EvictionPolicy::LRU => self.result_cache.evict_lru().await,
            EvictionPolicy::LFU => self.result_cache.evict_lfu().await,
            EvictionPolicy::FIFO => self.result_cache.evict_fifo().await,
            EvictionPolicy::Random => self.result_cache.evict_random().await,
        }

        self.cache_stats.record_result_eviction().await;
        Ok(())
    }

    async fn evict_metadata(&self) -> Result<(), AppError> {
        match self.eviction_policy {
            EvictionPolicy::LRU => self.metadata_cache.evict_lru().await,
            EvictionPolicy::LFU => self.metadata_cache.evict_lfu().await,
            EvictionPolicy::FIFO => self.metadata_cache.evict_fifo().await,
            EvictionPolicy::Random => self.metadata_cache.evict_random().await,
        }

        self.cache_stats.record_metadata_eviction().await;
        Ok(())
    }

    async fn start_cleanup_task(&self) {
        let document_cache = self.document_cache.clone();
        let result_cache = self.result_cache.clone();
        let metadata_cache = self.metadata_cache.clone();
        let stats = self.cache_stats.clone();
        let cleanup_interval = self.config.cleanup_interval;

        tokio::spawn(async move {
            let mut interval = interval(cleanup_interval);

            loop {
                interval.tick().await;

                let expired_count = document_cache.remove_expired().await
                    + result_cache.remove_expired().await
                    + metadata_cache.remove_expired().await;

                if expired_count > 0 {
                    stats.record_expired_cleanup(expired_count).await;
                }
            }
        });
    }
}

#[async_trait::async_trait]
impl PerformanceOptimizable for CacheManager {
    async fn optimize(&self) -> Result<(), AppError> {
        // 清理过期缓存
        self.clear_expired().await?;

        // 执行智能预加载
        self.smart_preload().await?;

        // 优化缓存分布
        self.optimize_cache_distribution().await?;

        Ok(())
    }

    async fn get_stats(&self) -> Result<serde_json::Value, AppError> {
        let stats = self.get_cache_stats().await?;
        let usage = self.get_cache_usage().await;

        Ok(serde_json::json!({
            "stats": stats,
            "usage": usage
        }))
    }

    async fn reset_stats(&self) -> Result<(), AppError> {
        self.cache_stats.reset().await;
        Ok(())
    }
}

impl CacheManager {
    async fn optimize_cache_distribution(&self) -> Result<(), AppError> {
        // 分析缓存使用模式
        let document_usage = self.document_cache.get_usage().await;
        let result_usage = self.result_cache.get_usage().await;
        let _metadata_usage = self.metadata_cache.get_usage().await;

        // 根据使用情况调整缓存大小
        if document_usage.hit_rate < 0.5 && document_usage.size > 100 {
            // 文档缓存命中率低，减少大小
            let new_size = (document_usage.capacity as f64 * 0.8) as usize;
            self.document_cache.resize(new_size).await?;
        }

        if result_usage.hit_rate > 0.9 && result_usage.size == result_usage.capacity {
            // 结果缓存命中率高且已满，增加大小
            let new_size = (result_usage.capacity as f64 * 1.2) as usize;
            self.result_cache.resize(new_size).await?;
        }

        Ok(())
    }
}

/// 文档缓存
pub struct DocumentCache {
    cache: DashMap<String, CacheEntry<Vec<u8>>>,
    max_size: AtomicUsize,
    access_order: Arc<Mutex<Vec<String>>>,
    access_count: DashMap<String, AtomicU64>,
}

impl DocumentCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: DashMap::new(),
            max_size: AtomicUsize::new(max_size),
            access_order: Arc::new(Mutex::new(Vec::new())),
            access_count: DashMap::new(),
        }
    }

    pub async fn insert(&self, key: String, entry: CacheEntry<Vec<u8>>) {
        self.cache.insert(key.clone(), entry);
        self.access_count.insert(key.clone(), AtomicU64::new(1));

        let mut order = self.access_order.lock().await;
        order.push(key);
    }

    pub async fn get(&self, key: &str) -> Option<CacheEntry<Vec<u8>>> {
        if let Some(entry) = self.cache.get(key) {
            // 更新访问计数
            if let Some(count) = self.access_count.get(key) {
                count.fetch_add(1, Ordering::Relaxed);
            }

            // 更新LRU顺序
            let mut order = self.access_order.lock().await;
            if let Some(pos) = order.iter().position(|k| k == key) {
                let key = order.remove(pos);
                order.push(key);
            }

            Some(entry.clone())
        } else {
            None
        }
    }

    pub async fn remove(&self, key: &str) {
        self.cache.remove(key);
        self.access_count.remove(key);

        let mut order = self.access_order.lock().await;
        if let Some(pos) = order.iter().position(|k| k == key) {
            order.remove(pos);
        }
    }

    pub async fn clear(&self) {
        self.cache.clear();
        self.access_count.clear();
        self.access_order.lock().await.clear();
    }

    pub async fn should_evict(&self) -> bool {
        self.cache.len() >= self.max_size.load(Ordering::Relaxed)
    }

    pub async fn evict_lru(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_lfu(&self) {
        let mut min_count = u64::MAX;
        let mut lfu_key = None;

        for entry in self.access_count.iter() {
            let count = entry.value().load(Ordering::Relaxed);
            if count < min_count {
                min_count = count;
                lfu_key = Some(entry.key().clone());
            }
        }

        if let Some(key) = lfu_key {
            self.remove(&key).await;
        }
    }

    pub async fn evict_fifo(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_random(&self) {
        if let Some(entry) = self.cache.iter().next() {
            let key = entry.key().clone();
            self.remove(&key).await;
        }
    }

    pub async fn remove_expired(&self) -> usize {
        let mut expired_keys = Vec::new();

        for entry in self.cache.iter() {
            if entry.value().is_expired() {
                expired_keys.push(entry.key().clone());
            }
        }

        let count = expired_keys.len();
        for key in expired_keys {
            self.remove(&key).await;
        }

        count
    }

    pub async fn get_usage(&self) -> CacheUsageInfo {
        let size = self.cache.len();
        let capacity = self.max_size.load(Ordering::Relaxed);

        // 计算命中率需要额外的统计信息
        CacheUsageInfo {
            size,
            capacity,
            hit_rate: 0.0,             // 需要从统计中获取
            memory_usage: size * 1024, // 估算
        }
    }

    pub async fn resize(&self, new_size: usize) -> Result<(), AppError> {
        let old_size = self.max_size.swap(new_size, Ordering::Relaxed);

        // 如果新大小更小，需要驱逐一些条目
        if new_size < old_size {
            while self.cache.len() > new_size {
                self.evict_lru().await;
            }
        }

        Ok(())
    }
}

/// 结果缓存（类似于DocumentCache的实现）
pub struct ResultCache {
    cache: DashMap<String, CacheEntry<ParseResult>>,
    max_size: AtomicUsize,
    access_order: Arc<Mutex<Vec<String>>>,
    access_count: DashMap<String, AtomicU64>,
}

impl ResultCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: DashMap::new(),
            max_size: AtomicUsize::new(max_size),
            access_order: Arc::new(Mutex::new(Vec::new())),
            access_count: DashMap::new(),
        }
    }

    // 实现与DocumentCache类似的方法
    pub async fn insert(&self, key: String, entry: CacheEntry<ParseResult>) {
        self.cache.insert(key.clone(), entry);
        self.access_count.insert(key.clone(), AtomicU64::new(1));

        let mut order = self.access_order.lock().await;
        order.push(key);
    }

    pub async fn get(&self, key: &str) -> Option<CacheEntry<ParseResult>> {
        if let Some(entry) = self.cache.get(key) {
            if let Some(count) = self.access_count.get(key) {
                count.fetch_add(1, Ordering::Relaxed);
            }

            let mut order = self.access_order.lock().await;
            if let Some(pos) = order.iter().position(|k| k == key) {
                let key = order.remove(pos);
                order.push(key);
            }

            Some(entry.clone())
        } else {
            None
        }
    }

    pub async fn remove(&self, key: &str) {
        self.cache.remove(key);
        self.access_count.remove(key);

        let mut order = self.access_order.lock().await;
        if let Some(pos) = order.iter().position(|k| k == key) {
            order.remove(pos);
        }
    }

    pub async fn clear(&self) {
        self.cache.clear();
        self.access_count.clear();
        self.access_order.lock().await.clear();
    }

    pub async fn should_evict(&self) -> bool {
        self.cache.len() >= self.max_size.load(Ordering::Relaxed)
    }

    pub async fn evict_lru(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_lfu(&self) {
        let mut min_count = u64::MAX;
        let mut lfu_key = None;

        for entry in self.access_count.iter() {
            let count = entry.value().load(Ordering::Relaxed);
            if count < min_count {
                min_count = count;
                lfu_key = Some(entry.key().clone());
            }
        }

        if let Some(key) = lfu_key {
            self.remove(&key).await;
        }
    }

    pub async fn evict_fifo(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_random(&self) {
        if let Some(entry) = self.cache.iter().next() {
            let key = entry.key().clone();
            self.remove(&key).await;
        }
    }

    pub async fn remove_expired(&self) -> usize {
        let mut expired_keys = Vec::new();

        for entry in self.cache.iter() {
            if entry.value().is_expired() {
                expired_keys.push(entry.key().clone());
            }
        }

        let count = expired_keys.len();
        for key in expired_keys {
            self.remove(&key).await;
        }

        count
    }

    pub async fn get_usage(&self) -> CacheUsageInfo {
        let size = self.cache.len();
        let capacity = self.max_size.load(Ordering::Relaxed);

        CacheUsageInfo {
            size,
            capacity,
            hit_rate: 0.0,
            memory_usage: size * 512, // 估算
        }
    }

    pub async fn resize(&self, new_size: usize) -> Result<(), AppError> {
        let old_size = self.max_size.swap(new_size, Ordering::Relaxed);

        if new_size < old_size {
            while self.cache.len() > new_size {
                self.evict_lru().await;
            }
        }

        Ok(())
    }
}

/// 元数据缓存（类似实现）
pub struct MetadataCache {
    cache: DashMap<String, CacheEntry<DocumentMetadata>>,
    max_size: AtomicUsize,
    access_order: Arc<Mutex<Vec<String>>>,
    access_count: DashMap<String, AtomicU64>,
}

impl MetadataCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: DashMap::new(),
            max_size: AtomicUsize::new(max_size),
            access_order: Arc::new(Mutex::new(Vec::new())),
            access_count: DashMap::new(),
        }
    }

    // 类似的方法实现...
    pub async fn insert(&self, key: String, entry: CacheEntry<DocumentMetadata>) {
        self.cache.insert(key.clone(), entry);
        self.access_count.insert(key.clone(), AtomicU64::new(1));

        let mut order = self.access_order.lock().await;
        order.push(key);
    }

    pub async fn get(&self, key: &str) -> Option<CacheEntry<DocumentMetadata>> {
        if let Some(entry) = self.cache.get(key) {
            if let Some(count) = self.access_count.get(key) {
                count.fetch_add(1, Ordering::Relaxed);
            }

            let mut order = self.access_order.lock().await;
            if let Some(pos) = order.iter().position(|k| k == key) {
                let key = order.remove(pos);
                order.push(key);
            }

            Some(entry.clone())
        } else {
            None
        }
    }

    pub async fn remove(&self, key: &str) {
        self.cache.remove(key);
        self.access_count.remove(key);

        let mut order = self.access_order.lock().await;
        if let Some(pos) = order.iter().position(|k| k == key) {
            order.remove(pos);
        }
    }

    pub async fn clear(&self) {
        self.cache.clear();
        self.access_count.clear();
        self.access_order.lock().await.clear();
    }

    pub async fn should_evict(&self) -> bool {
        self.cache.len() >= self.max_size.load(Ordering::Relaxed)
    }

    pub async fn evict_lru(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_lfu(&self) {
        let mut min_count = u64::MAX;
        let mut lfu_key = None;

        for entry in self.access_count.iter() {
            let count = entry.value().load(Ordering::Relaxed);
            if count < min_count {
                min_count = count;
                lfu_key = Some(entry.key().clone());
            }
        }

        if let Some(key) = lfu_key {
            self.remove(&key).await;
        }
    }

    pub async fn evict_fifo(&self) {
        let mut order = self.access_order.lock().await;
        if let Some(key) = order.first().cloned() {
            self.cache.remove(&key);
            self.access_count.remove(&key);
            order.remove(0);
        }
    }

    pub async fn evict_random(&self) {
        if let Some(entry) = self.cache.iter().next() {
            let key = entry.key().clone();
            self.remove(&key).await;
        }
    }

    pub async fn remove_expired(&self) -> usize {
        let mut expired_keys = Vec::new();

        for entry in self.cache.iter() {
            if entry.value().is_expired() {
                expired_keys.push(entry.key().clone());
            }
        }

        let count = expired_keys.len();
        for key in expired_keys {
            self.remove(&key).await;
        }

        count
    }

    pub async fn get_usage(&self) -> CacheUsageInfo {
        let size = self.cache.len();
        let capacity = self.max_size.load(Ordering::Relaxed);

        CacheUsageInfo {
            size,
            capacity,
            hit_rate: 0.0,
            memory_usage: size * 256, // 估算
        }
    }

    pub async fn resize(&self, new_size: usize) -> Result<(), AppError> {
        let old_size = self.max_size.swap(new_size, Ordering::Relaxed);

        if new_size < old_size {
            while self.cache.len() > new_size {
                self.evict_lru().await;
            }
        }

        Ok(())
    }
}

/// 缓存预加载器
pub struct CachePreloader {
    document_cache: Arc<DocumentCache>,
    preload_stats: Arc<PreloadStats>,
}

impl CachePreloader {
    pub async fn new(document_cache: Arc<DocumentCache>) -> Result<Self, AppError> {
        Ok(Self {
            document_cache,
            preload_stats: Arc::new(PreloadStats::new()),
        })
    }

    /// 预热指定文档
    pub async fn warmup(&self, documents: Vec<String>) -> Result<(), AppError> {
        for doc_path in documents {
            if let Ok(content) = tokio::fs::read(&doc_path).await {
                let cache_key = format!("preload:{doc_path}");
                let entry = CacheEntry::new(content, Duration::from_secs(3600));
                self.document_cache.insert(cache_key, entry).await;
                self.preload_stats.record_preload().await;
            }
        }

        Ok(())
    }

    /// 智能预加载
    pub async fn smart_preload(&self) -> Result<(), AppError> {
        // 基于访问模式的智能预加载逻辑
        // 这里可以实现机器学习算法来预测哪些文档可能被访问

        self.preload_stats.record_smart_preload().await;

        Ok(())
    }
}

/// 缓存条目
#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    pub data: T,
    pub created_at: Instant,
    pub ttl: Duration,
    pub access_count: u64,
    pub last_accessed: Instant,
}

impl<T> CacheEntry<T> {
    pub fn new(data: T, ttl: Duration) -> Self {
        let now = Instant::now();
        Self {
            data,
            created_at: now,
            ttl,
            access_count: 0,
            last_accessed: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }

    pub fn touch(&mut self) {
        self.access_count += 1;
        self.last_accessed = Instant::now();
    }
}

/// 解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub content: String,
    pub metadata: HashMap<String, String>,
    pub format: DocumentFormat,
    pub processing_time: Duration,
    pub created_at: SystemTime,
}

/// 文档元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub file_name: String,
    pub file_size: u64,
    pub format: DocumentFormat,
    pub created_at: SystemTime,
    pub modified_at: SystemTime,
    pub checksum: String,
    pub properties: HashMap<String, String>,
}

/// 驱逐策略
#[derive(Debug, Clone, Copy)]
pub enum EvictionPolicy {
    LRU,    // 最近最少使用
    LFU,    // 最少使用频率
    FIFO,   // 先进先出
    Random, // 随机
}

/// 缓存类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CacheType {
    Document,
    Result,
    Metadata,
}

/// 缓存统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub document_cache_hits: u64,
    pub document_cache_misses: u64,
    pub document_cache_writes: u64,
    pub document_evictions: u64,

    pub result_cache_hits: u64,
    pub result_cache_misses: u64,
    pub result_cache_writes: u64,
    pub result_evictions: u64,

    pub metadata_cache_hits: u64,
    pub metadata_cache_misses: u64,
    pub metadata_cache_writes: u64,
    pub metadata_evictions: u64,

    pub cache_clears: u64,
    pub expired_cleanups: u64,
    pub cache_resizes: u64,
}

impl Default for CacheStats {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheStats {
    pub fn new() -> Self {
        Self {
            document_cache_hits: 0,
            document_cache_misses: 0,
            document_cache_writes: 0,
            document_evictions: 0,
            result_cache_hits: 0,
            result_cache_misses: 0,
            result_cache_writes: 0,
            result_evictions: 0,
            metadata_cache_hits: 0,
            metadata_cache_misses: 0,
            metadata_cache_writes: 0,
            metadata_evictions: 0,
            cache_clears: 0,
            expired_cleanups: 0,
            cache_resizes: 0,
        }
    }

    // 记录方法（使用原子操作）
    pub async fn record_document_cache_hit(&self) {
        // 原子操作实现
    }

    pub async fn record_document_cache_miss(&self) {
        // 原子操作实现
    }

    pub async fn record_document_cache_write(&self) {
        // 原子操作实现
    }

    pub async fn record_document_eviction(&self) {
        // 原子操作实现
    }

    pub async fn record_result_cache_hit(&self) {
        // 原子操作实现
    }

    pub async fn record_result_cache_miss(&self) {
        // 原子操作实现
    }

    pub async fn record_result_cache_write(&self) {
        // 原子操作实现
    }

    pub async fn record_result_eviction(&self) {
        // 原子操作实现
    }

    pub async fn record_metadata_cache_hit(&self) {
        // 原子操作实现
    }

    pub async fn record_metadata_cache_miss(&self) {
        // 原子操作实现
    }

    pub async fn record_metadata_cache_write(&self) {
        // 原子操作实现
    }

    pub async fn record_metadata_eviction(&self) {
        // 原子操作实现
    }

    pub async fn record_cache_clear(&self) {
        // 原子操作实现
    }

    pub async fn record_expired_cleanup(&self, _count: usize) {
        // 原子操作实现
    }

    pub async fn record_cache_resize(&self, _cache_type: CacheType, _new_size: usize) {
        // 原子操作实现
    }

    pub async fn clone_stats(&self) -> CacheStats {
        self.clone()
    }

    pub async fn reset(&self) {
        // 重置所有统计数据
    }
}

/// 缓存使用情况
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheUsage {
    pub document_cache: CacheUsageInfo,
    pub result_cache: CacheUsageInfo,
    pub metadata_cache: CacheUsageInfo,
}

/// 缓存使用信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheUsageInfo {
    pub size: usize,
    pub capacity: usize,
    pub hit_rate: f64,
    pub memory_usage: usize,
}

/// 预加载统计
struct PreloadStats {
    preloads: AtomicU64,
    smart_preloads: AtomicU64,
}

impl PreloadStats {
    fn new() -> Self {
        Self {
            preloads: AtomicU64::new(0),
            smart_preloads: AtomicU64::new(0),
        }
    }

    async fn record_preload(&self) {
        self.preloads.fetch_add(1, Ordering::Relaxed);
    }

    async fn record_smart_preload(&self) {
        self.smart_preloads.fetch_add(1, Ordering::Relaxed);
    }
}
