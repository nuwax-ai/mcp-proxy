//! 内存优化器
//!
//! 提供内存使用监控、内存池管理和内存压缩功能

use dashmap::DashMap;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tokio_util::bytes::BytesMut;

use super::{MemoryConfig, PerformanceOptimizable};
use crate::config::AppConfig;
use crate::error::AppError;

/// 内存优化器
pub struct MemoryOptimizer {
    config: MemoryConfig,
    memory_pool: Arc<MemoryPool>,
    compression_manager: Arc<CompressionManager>,
    memory_monitor: Arc<MemoryMonitor>,
    stats: Arc<MemoryStats>,
}

impl MemoryOptimizer {
    /// 创建新的内存优化器
    pub async fn new(_config: &AppConfig) -> Result<Self, AppError> {
        let memory_config = MemoryConfig::default(); // 从配置中获取

        let memory_pool = Arc::new(MemoryPool::new(memory_config.pool_size));
        let compression_manager =
            Arc::new(CompressionManager::new(memory_config.enable_compression));
        let memory_monitor = Arc::new(MemoryMonitor::new(memory_config.max_memory_usage));
        let stats = Arc::new(MemoryStats::new());

        Ok(Self {
            config: memory_config,
            memory_pool,
            compression_manager,
            memory_monitor,
            stats,
        })
    }

    /// 分配内存
    pub async fn allocate(&self, size: usize) -> Result<MemoryBlock, AppError> {
        // 检查内存限制
        if !self.memory_monitor.can_allocate(size).await {
            // 尝试清理内存
            self.cleanup_memory().await?;

            // 再次检查
            if !self.memory_monitor.can_allocate(size).await {
                return Err(AppError::Config(format!(
                    "Memory limit exceeded: requested {} bytes, available {} bytes",
                    size,
                    self.memory_monitor.available_memory().await
                )));
            }
        }

        // 从内存池分配
        let block = self.memory_pool.allocate(size).await?;

        // 更新统计
        self.stats.record_allocation(size);
        self.memory_monitor.record_allocation(size).await;

        Ok(block)
    }

    /// 释放内存
    pub async fn deallocate(&self, block: MemoryBlock) -> Result<(), AppError> {
        let size = block.size();

        // 返回到内存池
        self.memory_pool.deallocate(block).await?;

        // 更新统计
        self.stats.record_deallocation(size);
        self.memory_monitor.record_deallocation(size).await;

        Ok(())
    }

    /// 压缩数据
    pub async fn compress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if !self.config.enable_compression {
            return Ok(data.to_vec());
        }

        self.compression_manager.compress(data).await
    }

    /// 解压数据
    pub async fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if !self.config.enable_compression {
            return Ok(data.to_vec());
        }

        self.compression_manager.decompress(data).await
    }

    /// 清理内存
    pub async fn cleanup_memory(&self) -> Result<(), AppError> {
        // 清理内存池
        self.memory_pool.cleanup().await?;

        // 强制垃圾回收（如果可能）
        #[cfg(target_os = "linux")]
        {
            // 在 Linux 上尝试将空闲内存归还给系统
            unsafe {
                libc::malloc_trim(0);
            }
        }

        self.stats.record_cleanup();

        Ok(())
    }

    /// 获取内存统计
    pub async fn get_memory_stats(&self) -> Result<MemoryStats, AppError> {
        Ok(self.stats.clone_stats().await)
    }

    /// 获取内存使用情况
    pub async fn get_memory_usage(&self) -> Result<MemoryUsage, AppError> {
        Ok(MemoryUsage {
            total_allocated: self.memory_monitor.total_allocated().await,
            total_available: self.memory_monitor.available_memory().await,
            pool_usage: self.memory_pool.usage().await,
            compression_ratio: self.compression_manager.compression_ratio().await,
        })
    }
}

#[async_trait::async_trait]
impl PerformanceOptimizable for MemoryOptimizer {
    async fn optimize(&self) -> Result<(), AppError> {
        // 检查内存使用情况
        let usage = self.get_memory_usage().await?;
        let usage_ratio = usage.total_allocated as f64 / usage.total_available as f64;

        // 如果内存使用超过阈值，执行清理
        if usage_ratio > self.config.cleanup_threshold {
            self.cleanup_memory().await?;
        }

        // 优化内存池
        self.memory_pool.optimize().await?;

        Ok(())
    }

    async fn get_stats(&self) -> Result<serde_json::Value, AppError> {
        let stats = self.get_memory_stats().await?;
        let usage = self.get_memory_usage().await?;

        Ok(serde_json::json!({
            "stats": stats,
            "usage": usage
        }))
    }

    async fn reset_stats(&self) -> Result<(), AppError> {
        self.stats.reset().await;
        Ok(())
    }
}

/// 内存池
pub struct MemoryPool {
    pools: DashMap<usize, Arc<Mutex<VecDeque<MemoryBlock>>>>,
    max_pool_size: usize,
    stats: Arc<PoolStats>,
}

impl MemoryPool {
    pub fn new(max_pool_size: usize) -> Self {
        Self {
            pools: DashMap::new(),
            max_pool_size,
            stats: Arc::new(PoolStats::new()),
        }
    }

    pub async fn allocate(&self, size: usize) -> Result<MemoryBlock, AppError> {
        // 计算合适的块大小（2的幂次）
        let block_size = self.calculate_block_size(size);

        // 尝试从池中获取
        if let Some(pool) = self.pools.get(&block_size) {
            let mut pool_guard = pool.lock().await;
            if let Some(block) = pool_guard.pop_front() {
                self.stats.record_pool_hit();
                return Ok(block);
            }
        }

        // 池中没有可用块，创建新块
        let block = MemoryBlock::new(block_size)?;
        self.stats.record_pool_miss();

        Ok(block)
    }

    pub async fn deallocate(&self, block: MemoryBlock) -> Result<(), AppError> {
        let block_size = block.size();

        // 获取或创建对应大小的池
        let pool = self
            .pools
            .entry(block_size)
            .or_insert_with(|| Arc::new(Mutex::new(VecDeque::new())))
            .clone();

        let mut pool_guard = pool.lock().await;

        // 如果池未满，将块返回到池中
        if pool_guard.len() < self.max_pool_size {
            pool_guard.push_back(block);
            self.stats.record_pool_return();
        } else {
            // 池已满，直接丢弃块
            drop(block);
            self.stats.record_pool_discard();
        }

        Ok(())
    }

    pub async fn cleanup(&self) -> Result<(), AppError> {
        // 清理所有池中的一半块
        for entry in self.pools.iter() {
            let pool = entry.value().clone();
            let mut pool_guard = pool.lock().await;
            let current_size = pool_guard.len();
            let target_size = current_size / 2;

            while pool_guard.len() > target_size {
                pool_guard.pop_back();
            }
        }

        self.stats.record_cleanup();
        Ok(())
    }

    pub async fn optimize(&self) -> Result<(), AppError> {
        // 移除空的池
        self.pools.retain(|_, pool| {
            if let Ok(guard) = pool.try_lock() {
                !guard.is_empty()
            } else {
                true // 如果无法获取锁，保留池
            }
        });

        Ok(())
    }

    pub async fn usage(&self) -> PoolUsage {
        let mut total_blocks = 0;
        let mut total_memory = 0;

        for entry in self.pools.iter() {
            let block_size = *entry.key();
            if let Ok(pool_guard) = entry.value().try_lock() {
                let count = pool_guard.len();
                total_blocks += count;
                total_memory += count * block_size;
            }
        }

        PoolUsage {
            total_pools: self.pools.len(),
            total_blocks,
            total_memory,
            stats: self.stats.get_stats().await,
        }
    }

    fn calculate_block_size(&self, size: usize) -> usize {
        // 向上舍入到最近的2的幂次
        let mut block_size = 1;
        while block_size < size {
            block_size <<= 1;
        }
        block_size.max(64) // 最小64字节
    }
}

/// 内存块
pub struct MemoryBlock {
    data: BytesMut,
    size: usize,
    allocated_at: Instant,
}

impl MemoryBlock {
    pub fn new(size: usize) -> Result<Self, AppError> {
        let data = BytesMut::with_capacity(size);

        Ok(Self {
            data,
            size,
            allocated_at: Instant::now(),
        })
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut BytesMut {
        &mut self.data
    }

    pub fn age(&self) -> Duration {
        self.allocated_at.elapsed()
    }
}

/// 压缩管理器
pub struct CompressionManager {
    enabled: bool,
    compression_level: Compression,
    stats: Arc<CompressionStats>,
}

impl CompressionManager {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            compression_level: Compression::default(),
            stats: Arc::new(CompressionStats::new()),
        }
    }

    pub async fn compress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if !self.enabled {
            return Ok(data.to_vec());
        }

        let start = Instant::now();
        let original_size = data.len();

        let mut encoder = GzEncoder::new(Vec::new(), self.compression_level);
        encoder.write_all(data)?;

        let compressed = encoder
            .finish()
            .map_err(|e| AppError::Config(format!("Compression error: {e}")))?;

        let compressed_size = compressed.len();
        let duration = start.elapsed();

        self.stats
            .record_compression(original_size, compressed_size, duration)
            .await;

        Ok(compressed)
    }

    pub async fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if !self.enabled {
            return Ok(data.to_vec());
        }

        let start = Instant::now();
        let compressed_size = data.len();

        let mut decoder = GzDecoder::new(data);
        let mut decompressed = Vec::new();

        decoder.read_to_end(&mut decompressed)?;

        let decompressed_size = decompressed.len();
        let duration = start.elapsed();

        self.stats
            .record_decompression(compressed_size, decompressed_size, duration)
            .await;

        Ok(decompressed)
    }

    pub async fn compression_ratio(&self) -> f64 {
        self.stats.average_compression_ratio().await
    }
}

/// 内存监控器
pub struct MemoryMonitor {
    max_memory: u64,
    current_allocated: AtomicU64,
    peak_allocated: AtomicU64,
    allocation_count: AtomicUsize,
    deallocation_count: AtomicUsize,
}

impl MemoryMonitor {
    pub fn new(max_memory: u64) -> Self {
        Self {
            max_memory,
            current_allocated: AtomicU64::new(0),
            peak_allocated: AtomicU64::new(0),
            allocation_count: AtomicUsize::new(0),
            deallocation_count: AtomicUsize::new(0),
        }
    }

    pub async fn can_allocate(&self, size: usize) -> bool {
        let current = self.current_allocated.load(Ordering::Relaxed);
        current + size as u64 <= self.max_memory
    }

    pub async fn record_allocation(&self, size: usize) {
        let new_allocated = self
            .current_allocated
            .fetch_add(size as u64, Ordering::Relaxed)
            + size as u64;

        // 更新峰值
        let mut peak = self.peak_allocated.load(Ordering::Relaxed);
        while new_allocated > peak {
            match self.peak_allocated.compare_exchange_weak(
                peak,
                new_allocated,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current_peak) => peak = current_peak,
            }
        }

        self.allocation_count.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn record_deallocation(&self, size: usize) {
        self.current_allocated
            .fetch_sub(size as u64, Ordering::Relaxed);
        self.deallocation_count.fetch_add(1, Ordering::Relaxed);
    }

    pub async fn total_allocated(&self) -> u64 {
        self.current_allocated.load(Ordering::Relaxed)
    }

    pub async fn available_memory(&self) -> u64 {
        let current = self.current_allocated.load(Ordering::Relaxed);
        self.max_memory.saturating_sub(current)
    }

    pub async fn peak_memory(&self) -> u64 {
        self.peak_allocated.load(Ordering::Relaxed)
    }
}

/// 内存统计
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryStats {
    pub total_allocations: u64,
    pub total_deallocations: u64,
    pub current_allocated: u64,
    pub peak_allocated: u64,
    pub cleanup_count: u64,
    pub compression_stats: CompressionStatsData,
    pub pool_stats: PoolStatsData,
}

impl Default for MemoryStats {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStats {
    pub fn new() -> Self {
        Self {
            total_allocations: 0,
            total_deallocations: 0,
            current_allocated: 0,
            peak_allocated: 0,
            cleanup_count: 0,
            compression_stats: CompressionStatsData::new(),
            pool_stats: PoolStatsData::new(),
        }
    }

    pub fn record_allocation(&self, _size: usize) {
        // 在实际实现中，这些应该是原子操作
    }

    pub fn record_deallocation(&self, _size: usize) {
        // 在实际实现中，这些应该是原子操作
    }

    pub fn record_cleanup(&self) {
        // 在实际实现中，这些应该是原子操作
    }

    pub async fn clone_stats(&self) -> MemoryStats {
        self.clone()
    }

    pub async fn reset(&self) {
        // 重置统计数据
    }
}

/// 其他统计结构
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryUsage {
    pub total_allocated: u64,
    pub total_available: u64,
    pub pool_usage: PoolUsage,
    pub compression_ratio: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolUsage {
    pub total_pools: usize,
    pub total_blocks: usize,
    pub total_memory: usize,
    pub stats: PoolStatsData,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolStatsData {
    pub hits: u64,
    pub misses: u64,
    pub returns: u64,
    pub discards: u64,
    pub cleanups: u64,
}

impl Default for PoolStatsData {
    fn default() -> Self {
        Self::new()
    }
}

impl PoolStatsData {
    pub fn new() -> Self {
        Self {
            hits: 0,
            misses: 0,
            returns: 0,
            discards: 0,
            cleanups: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompressionStatsData {
    pub compressions: u64,
    pub decompressions: u64,
    pub total_original_size: u64,
    pub total_compressed_size: u64,
    pub average_compression_time: Duration,
    pub average_decompression_time: Duration,
}

impl Default for CompressionStatsData {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionStatsData {
    pub fn new() -> Self {
        Self {
            compressions: 0,
            decompressions: 0,
            total_original_size: 0,
            total_compressed_size: 0,
            average_compression_time: Duration::from_secs(0),
            average_decompression_time: Duration::from_secs(0),
        }
    }
}

// 辅助结构的实现
struct PoolStats {
    hits: AtomicU64,
    misses: AtomicU64,
    returns: AtomicU64,
    discards: AtomicU64,
    cleanups: AtomicU64,
}

impl PoolStats {
    fn new() -> Self {
        Self {
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            returns: AtomicU64::new(0),
            discards: AtomicU64::new(0),
            cleanups: AtomicU64::new(0),
        }
    }

    fn record_pool_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_pool_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    fn record_pool_return(&self) {
        self.returns.fetch_add(1, Ordering::Relaxed);
    }

    fn record_pool_discard(&self) {
        self.discards.fetch_add(1, Ordering::Relaxed);
    }

    fn record_cleanup(&self) {
        self.cleanups.fetch_add(1, Ordering::Relaxed);
    }

    async fn get_stats(&self) -> PoolStatsData {
        PoolStatsData {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            returns: self.returns.load(Ordering::Relaxed),
            discards: self.discards.load(Ordering::Relaxed),
            cleanups: self.cleanups.load(Ordering::Relaxed),
        }
    }
}

struct CompressionStats {
    compressions: AtomicU64,
    decompressions: AtomicU64,
    total_original_size: AtomicU64,
    total_compressed_size: AtomicU64,
    total_compression_time: RwLock<Duration>,
    total_decompression_time: RwLock<Duration>,
}

impl CompressionStats {
    fn new() -> Self {
        Self {
            compressions: AtomicU64::new(0),
            decompressions: AtomicU64::new(0),
            total_original_size: AtomicU64::new(0),
            total_compressed_size: AtomicU64::new(0),
            total_compression_time: RwLock::new(Duration::from_secs(0)),
            total_decompression_time: RwLock::new(Duration::from_secs(0)),
        }
    }

    async fn record_compression(
        &self,
        original_size: usize,
        compressed_size: usize,
        duration: Duration,
    ) {
        self.compressions.fetch_add(1, Ordering::Relaxed);
        self.total_original_size
            .fetch_add(original_size as u64, Ordering::Relaxed);
        self.total_compressed_size
            .fetch_add(compressed_size as u64, Ordering::Relaxed);

        let mut total_time = self.total_compression_time.write().await;
        *total_time += duration;
    }

    async fn record_decompression(
        &self,
        _compressed_size: usize,
        _decompressed_size: usize,
        duration: Duration,
    ) {
        self.decompressions.fetch_add(1, Ordering::Relaxed);

        let mut total_time = self.total_decompression_time.write().await;
        *total_time += duration;
    }

    async fn average_compression_ratio(&self) -> f64 {
        let original = self.total_original_size.load(Ordering::Relaxed);
        let compressed = self.total_compressed_size.load(Ordering::Relaxed);

        if original > 0 {
            compressed as f64 / original as f64
        } else {
            1.0
        }
    }
}
