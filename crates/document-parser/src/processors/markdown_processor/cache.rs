//! `MarkdownProcessor` 的缓存族方法（从 markdown_processor.rs 拆出）：缓存键
//! 生成、读写、清理与统计。纯代码搬移，无行为变化。

use crate::models::{DocumentStructure, TocItem};
use moka::future::Cache;
use std::time::Duration;

use super::CacheStatistics;

impl super::MarkdownProcessor {
    /// 生成缓存键
    pub(super) fn generate_cache_key(&self, content: &str) -> String {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hasher.finalize();
        format!("md_{}", hex::encode(hash))
    }

    /// 提取章节内容
    pub(super) fn extract_section_content(&self, content: &str, toc_item: &TocItem) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let start_line = toc_item.start_pos;
        let end_line = toc_item.end_pos.min(lines.len());

        if start_line < lines.len() && start_line < end_line {
            lines[start_line..end_line].join("\n")
        } else {
            // 如果位置信息不准确，尝试通过标题查找内容
            self.extract_content_by_title(content, &toc_item.title, toc_item.level)
        }
    }

    /// 通过标题提取内容
    fn extract_content_by_title(&self, content: &str, title: &str, level: u8) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let header_prefix = "#".repeat(level as usize);
        let target_header = format!("{header_prefix} {title}");

        let mut start_idx = None;
        let mut end_idx = lines.len();

        // 找到目标标题的位置
        for (i, line) in lines.iter().enumerate() {
            if line.trim() == target_header.trim() || line.trim().contains(title) {
                start_idx = Some(i + 1); // 从标题下一行开始
                break;
            }
        }

        if let Some(start) = start_idx {
            // 找到下一个同级或更高级标题的位置
            for (i, line) in lines.iter().enumerate().skip(start) {
                if line.starts_with('#') {
                    let line_level = line.chars().take_while(|&c| c == '#').count() as u8;
                    if line_level <= level {
                        end_idx = i;
                        break;
                    }
                }
            }

            lines[start..end_idx].join("\n")
        } else {
            format!("Content for: {title}")
        }
    }

    /// 从缓存获取
    pub(super) async fn get_from_cache(&self, key: &str) -> Option<DocumentStructure> {
        let cache = self.cache.lock().await;
        cache.get(key).await
    }

    /// 存储到缓存
    pub(super) async fn store_in_cache(&self, key: String, data: DocumentStructure) {
        let cache = self.cache.lock().await;
        cache.insert(key, data).await;
    }

    /// 清空缓存
    pub async fn clear_cache(&self) {
        // moka 缓存没有 clear 方法，我们重新创建一个新的缓存
        *self.cache.lock().await = Cache::builder()
            .max_capacity(self.config.max_cache_entries as u64)
            .time_to_live(Duration::from_secs(self.config.cache_ttl_seconds))
            .time_to_idle(Duration::from_secs(self.config.cache_ttl_seconds / 2))
            .build();
    }

    /// 获取缓存统计
    pub async fn get_cache_stats(&self) -> CacheStatistics {
        let cache = self.cache.lock().await;

        CacheStatistics {
            total_entries: cache.entry_count() as usize,
            expired_entries: 0, // moka 自动处理过期
            hit_rate: 0.0,      // 需要额外统计
            memory_usage_estimate: (cache.entry_count() * 1024) as usize, // 粗略估计
        }
    }
}
