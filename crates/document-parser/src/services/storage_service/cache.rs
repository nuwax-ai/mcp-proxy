//! `StorageService` 的缓存层方法（从 storage_service.rs 拆出）：任务内存缓存、
//! 查询过滤器匹配与缓存键、通用 sled 缓存读写、缓存失效与过期清理。纯代码搬移。

use crate::error::AppError;
use crate::models::DocumentTask;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{CACHE_PREFIX, CacheItem, METADATA_PREFIX, QueryFilter};

impl super::StorageService {
    /// 内存缓存操作
    pub(super) async fn get_from_memory_cache(&self, task_id: &str) -> Option<DocumentTask> {
        let cache = self.memory_cache.read().await;
        if let Some(cache_item) = cache.get(task_id) {
            // 检查是否过期
            if let Some(expires_at) = cache_item.expires_at
                && SystemTime::now() > expires_at
            {
                return None;
            }
            Some(cache_item.data.clone())
        } else {
            None
        }
    }

    pub(super) async fn update_memory_cache(&self, task_id: &str, task: DocumentTask) {
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

    pub(super) async fn remove_from_memory_cache(&self, task_id: &str) {
        let mut cache = self.memory_cache.write().await;
        cache.remove(task_id);
    }

    /// 检查任务是否匹配过滤器
    pub(super) fn task_matches_filter(&self, task: &DocumentTask, filter: &QueryFilter) -> bool {
        // 状态过滤
        if let Some(status) = &filter.status
            && &task.status != status
        {
            return false;
        }

        // 格式过滤
        if let Some(format) = &filter.format
            && task.document_format.as_ref() != Some(format)
        {
            return false;
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
    pub(super) fn filter_to_cache_key(&self, filter: &QueryFilter) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}",
            filter
                .status
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "any".to_string()),
            filter
                .format
                .as_ref()
                .map(|f| f.to_string())
                .unwrap_or_else(|| "any".to_string()),
            filter
                .created_after
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(0),
            filter
                .created_before
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs())
                .unwrap_or(u64::MAX),
            filter.limit.unwrap_or(100),
            filter.offset.unwrap_or(0)
        )
    }

    /// 从缓存获取数据
    pub(super) async fn get_from_cache<T>(&self, key: &str) -> Result<Option<T>, AppError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let cache_key = format!("{CACHE_PREFIX}{key}");

        if let Ok(Some(data)) = self.cache_tree.get(&cache_key) {
            let cache_item: CacheItem<T> = serde_json::from_slice(&data)
                .map_err(|e| AppError::Database(format!("反序列化缓存项失败: {e}")))?;

            // 检查是否过期
            if let Some(expires_at) = cache_item.expires_at
                && SystemTime::now() > expires_at
            {
                // 过期，删除缓存项
                self.cache_tree
                    .remove(&cache_key)
                    .map_err(|e| AppError::Database(format!("删除过期缓存失败: {e}")))?;
                return Ok(None);
            }

            // 更新访问计数（简化版本，不实际更新）
            Ok(Some(cache_item.data))
        } else {
            Ok(None)
        }
    }

    /// 设置缓存
    pub(super) async fn set_cache<T>(
        &self,
        key: &str,
        data: &T,
        ttl: Option<std::time::Duration>,
    ) -> Result<(), AppError>
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

        self.cache_tree
            .insert(&cache_key, cache_data)
            .map_err(|e| AppError::Database(format!("设置缓存失败: {e}")))?;

        Ok(())
    }

    /// 清除任务相关缓存
    pub(super) async fn invalidate_cache_for_task(&self, task_id: &str) -> Result<(), AppError> {
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
                    let (key, _) =
                        result.map_err(|e| AppError::Database(format!("扫描缓存失败: {e}")))?;
                    to_remove.push(key.to_vec());
                }

                for key in to_remove {
                    self.cache_tree
                        .remove(&key)
                        .map_err(|e| AppError::Database(format!("删除缓存失败: {e}")))?;
                }
            } else {
                self.cache_tree
                    .remove(&cache_key)
                    .map_err(|e| AppError::Database(format!("删除缓存失败: {e}")))?;
            }
        }

        Ok(())
    }

    /// 清理过期缓存
    pub(super) async fn cleanup_expired_cache(&self) -> Result<usize, AppError> {
        let mut cleaned_count = 0;
        let now = SystemTime::now();
        let mut to_remove = Vec::new();

        for result in self.cache_tree.scan_prefix(CACHE_PREFIX.as_bytes()) {
            let (key, data) =
                result.map_err(|e| AppError::Database(format!("扫描缓存失败: {e}")))?;

            // 尝试解析缓存项（简化版本）
            if let Ok(cache_item) = serde_json::from_slice::<serde_json::Value>(&data)
                && let Some(expires_at_timestamp) =
                    cache_item.get("expires_at").and_then(|v| v.as_u64())
            {
                let expires_at = UNIX_EPOCH + std::time::Duration::from_secs(expires_at_timestamp);
                if now > expires_at {
                    to_remove.push(key.to_vec());
                }
            }
        }

        for key in to_remove {
            self.cache_tree
                .remove(&key)
                .map_err(|e| AppError::Database(format!("删除过期缓存失败: {e}")))?;
            cleaned_count += 1;
        }

        Ok(cleaned_count)
    }

    /// 获取最后清理时间
    pub(super) async fn get_last_cleanup_time(&self) -> Result<Option<SystemTime>, AppError> {
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
    pub(super) async fn set_last_cleanup_time(&self, time: SystemTime) -> Result<(), AppError> {
        let key = format!("{}{}", METADATA_PREFIX, "last_cleanup");
        let timestamp = time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let data = serde_json::to_vec(&timestamp)
            .map_err(|e| AppError::Database(format!("序列化清理时间失败: {e}")))?;

        self.metadata_tree
            .insert(&key, data)
            .map_err(|e| AppError::Database(format!("设置清理时间失败: {e}")))?;

        Ok(())
    }
}
