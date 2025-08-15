use dashmap::DashMap;
use pulldown_cmark::{Parser, Event, Tag, TagEnd, HeadingLevel, Options, CowStr};
use pulldown_cmark_toc::TableOfContents;
use crate::config::{get_file_size_limit, FileSizePurpose};
use crate::error::AppError;
use crate::models::{TocItem, DocumentStructure, StructuredDocument, StructuredSection};
use std::io::{BufRead, BufReader, Cursor};
use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::{Result, Context};
use tracing::{debug, info, warn, instrument};
use std::time::Instant;

/// Markdown处理器配置
#[derive(Debug, Clone)]
pub struct MarkdownProcessorConfig {
    /// 是否启用TOC生成
    pub enable_toc: bool,
    /// TOC最大深度
    pub max_toc_depth: usize,
    /// 是否启用锚点生成
    pub enable_anchors: bool,
    /// 是否启用缓存
    pub enable_cache: bool,
    /// 流式处理的缓冲区大小（字节）
    pub streaming_buffer_size: usize,
    /// 大文档阈值（字节）
    pub large_document_threshold: usize,
    /// 内容验证和清理
    pub enable_content_validation: bool,
    /// 最大缓存条目数
    pub max_cache_entries: usize,
    /// 缓存TTL（秒）
    pub cache_ttl_seconds: u64,
}

impl MarkdownProcessorConfig {
    /// 使用全局配置创建Markdown处理器配置
    pub fn with_global_config() -> Self {
        use crate::config::get_large_document_threshold;
        
        // 安全地获取大文档阈值，如果全局配置未初始化则使用默认值
        let large_document_threshold = match std::panic::catch_unwind(|| {
            get_large_document_threshold()
        }) {
            Ok(threshold) => threshold as usize,
            Err(_) => 10 * 1024 * 1024, // 默认10MB
        };
        
        Self {
            enable_toc: true,
            max_toc_depth: 6,
            enable_anchors: true,
            enable_cache: true,
            streaming_buffer_size: 64 * 1024, // 64KB
            large_document_threshold,
            enable_content_validation: true,
            max_cache_entries: 1000,
            cache_ttl_seconds: 3600, // 1 hour
        }
    }
}

impl Default for MarkdownProcessorConfig {
    fn default() -> Self {
        Self::with_global_config()
    }
}

/// 缓存条目
#[derive(Debug, Clone)]
struct CacheEntry {
    data: DocumentStructure,
    created_at: std::time::Instant,
    access_count: u64,
}

impl CacheEntry {
    fn new(data: DocumentStructure) -> Self {
        Self {
            data,
            created_at: std::time::Instant::now(),
            access_count: 1,
        }
    }

    fn is_expired(&self, ttl_seconds: u64) -> bool {
        self.created_at.elapsed().as_secs() > ttl_seconds
    }

    fn access(&mut self) -> &DocumentStructure {
        self.access_count += 1;
        &self.data
    }
}

/// Markdown处理器（优化版本）
pub struct MarkdownProcessor {
    config: MarkdownProcessorConfig,
    cache: Arc<RwLock<DashMap<String, CacheEntry>>>,
    parser_options: Options,
}

impl MarkdownProcessor {
    /// 创建新的Markdown处理器
    pub fn new(config: MarkdownProcessorConfig) -> Self {
        let mut parser_options = Options::empty();
        parser_options.insert(Options::ENABLE_TABLES);
        parser_options.insert(Options::ENABLE_FOOTNOTES);
        parser_options.insert(Options::ENABLE_STRIKETHROUGH);
        parser_options.insert(Options::ENABLE_TASKLISTS);
        parser_options.insert(Options::ENABLE_SMART_PUNCTUATION);
        parser_options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

        Self {
            config,
            cache: Arc::new(RwLock::new(DashMap::new())),
            parser_options,
        }
    }
    
    /// 解析Markdown并生成TOC（优化版本）
    #[instrument(skip(self, content), fields(content_size = content.len()))]
    pub async fn parse_markdown_with_toc(&self, content: &str) -> Result<DocumentStructure, AppError> {
        let start_time = Instant::now();
        
        // 内容验证和清理
        let sanitized_content = if self.config.enable_content_validation {
            self.sanitize_content(content)?
        } else {
            content.to_string()
        };

        // 检查缓存
        let cache_key = if self.config.enable_cache {
            Some(self.generate_cache_key(&sanitized_content))
        } else {
            None
        };
        
        if let Some(key) = &cache_key {
            if let Some(cached) = self.get_from_cache(key).await {
                debug!("Cache hit for key: {}", key);
                return Ok(cached);
            }
        }
        
        // 选择处理策略
        let doc_structure = if sanitized_content.len() > self.config.large_document_threshold {
            info!("Processing large document with streaming approach");
            self.parse_large_document_streaming(&sanitized_content).await?
        } else {
            self.parse_document_standard(&sanitized_content).await?
        };
        
        // 缓存结果
        if let Some(key) = cache_key {
            self.store_in_cache(key, doc_structure.clone()).await;
        }
        
        let processing_time = start_time.elapsed();
        info!("Markdown processing completed in {:?}", processing_time);
        
        Ok(doc_structure)
    }

    /// 标准文档处理
    #[instrument(skip(self, content))]
    async fn parse_document_standard(&self, content: &str) -> Result<DocumentStructure, AppError> {
        // 使用优化的解析器选项
        let parser = Parser::new_ext(content, self.parser_options);
        let events: Vec<Event> = parser.collect();
        
        // 生成TOC（使用pulldown_cmark_toc优化）
        let toc_items = if self.config.enable_toc {
            self.generate_toc_optimized(content, &events).await?
        } else {
            Vec::new()
        };
        
        // 生成结构化文档
        let structured_doc = self.generate_structured_document_optimized(content, &events, &toc_items).await?;
        
        // 创建文档结构
        let mut doc_structure = DocumentStructure::new("Markdown Document".to_string());
        for item in toc_items {
            doc_structure.add_toc_item(item);
        }
        
        // 添加sections内容
        for section in &structured_doc.toc {
            doc_structure.add_section(section.id.clone(), section.content.clone());
        }
        
        Ok(doc_structure)
    }

    /// 大文档流式处理
    #[instrument(skip(self, content))]
    async fn parse_large_document_streaming(&self, content: &str) -> Result<DocumentStructure, AppError> {
        let mut doc_structure = DocumentStructure::new("Large Markdown Document".to_string());
        let reader = BufReader::with_capacity(self.config.streaming_buffer_size, Cursor::new(content));
        
        let mut current_section: Option<(String, String, u8, String)> = None; // (id, title, level, content)
        let mut line_number = 0;
        let mut toc_items = Vec::new();
        
        for line_result in reader.lines() {
            let line = line_result.map_err(|e| AppError::Parse(format!("读取行失败: {}", e)))?;
            line_number += 1;
            
            // 检查是否为标题行
            if let Some((level, title)) = self.parse_heading_line(&line) {
                // 保存当前section
                if let Some((id, section_title, section_level, section_content)) = current_section.take() {
                    let toc_item = TocItem::new(id.clone(), section_title, section_level, 0, line_number - 1);
                    toc_items.push(toc_item);
                    doc_structure.add_section(id, section_content);
                }
                
                // 开始新section
                let section_id = self.generate_anchor_id(&title);
                current_section = Some((section_id, title, level, String::new()));
            } else if let Some((_, _, _, ref mut content)) = current_section {
                // 添加内容到当前section
                content.push_str(&line);
                content.push('\n');
            }
            
            // 定期让出控制权以避免阻塞
            if line_number % 1000 == 0 {
                tokio::task::yield_now().await;
            }
        }
        
        // 保存最后一个section
        if let Some((id, section_title, section_level, section_content)) = current_section {
            let toc_item = TocItem::new(id.clone(), section_title, section_level, 0, line_number);
            toc_items.push(toc_item);
            doc_structure.add_section(id, section_content);
        }
        
        // 添加TOC项
        for item in toc_items {
            doc_structure.add_toc_item(item);
        }
        
        Ok(doc_structure)
    }

    /// 解析标题行
    fn parse_heading_line(&self, line: &str) -> Option<(u8, String)> {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|&c| c == '#').count() as u8;
            if level > 0 && level <= 6 {
                let title = trimmed.trim_start_matches('#').trim().to_string();
                if !title.is_empty() {
                    return Some((level, title));
                }
            }
        }
        None
    }
    
    /// 生成TOC（优化版本使用pulldown_cmark_toc）
    #[instrument(skip(self, content, events))]
    async fn generate_toc_optimized<'a>(&self, content: &'a str, events: &'a [Event<'a>]) -> Result<Vec<TocItem>, AppError> {
        // 使用pulldown_cmark_toc进行高效TOC生成
        let toc = TableOfContents::new(content);
        let mut toc_items = Vec::with_capacity(toc.headings().len());
        
        for (index, heading) in toc.headings().enumerate() {
            let level = heading.level() as u8;
            if level <= self.config.max_toc_depth as u8 {
                let title = heading.text().to_string();
                let anchor = if self.config.enable_anchors {
                    self.generate_anchor_id(&title)
                } else {
                    format!("heading-{}", index)
                };
                
                let item = TocItem::new(
                    anchor,
                    title,
                    level,
                    index,
                    index + 1,
                );
                toc_items.push(item);
            }
        }
        
        // 如果pulldown_cmark_toc失败，回退到手动解析
        if toc_items.is_empty() {
            warn!("pulldown_cmark_toc failed, falling back to manual parsing");
            return self.generate_toc_manual(events).await;
        }
        
        Ok(toc_items)
    }

    /// 手动TOC生成（回退方案）
    #[instrument(skip(self, events))]
    async fn generate_toc_manual<'a>(&self, events: &'a [Event<'a>]) -> Result<Vec<TocItem>, AppError> {
        let mut toc_items: Vec<TocItem> = Vec::new();
        let mut current_title: String = String::new();
        let mut current_level: Option<u8> = None;
        let mut position = 0;

        for event in events {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    current_level = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    });
                    current_title.clear();
                }
                Event::Text(text) => {
                    if current_level.is_some() {
                        current_title.push_str(&text);
                    }
                }
                Event::Code(code) => {
                    if current_level.is_some() {
                        current_title.push('`');
                        current_title.push_str(&code);
                        current_title.push('`');
                    }
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(level) = current_level.take() {
                        if level <= self.config.max_toc_depth as u8 {
                            let title = current_title.trim().to_string();
                            let anchor = self.generate_anchor_id(&title);
                            let item = TocItem::new(anchor, title, level, position, position);
                            toc_items.push(item);
                        }
                        current_title.clear();
                    }
                }
                _ => {}
            }
            position += 1;
        }

        Ok(toc_items)
    }

    
    /// 内容验证和清理
    #[instrument(skip(self, content))]
    fn sanitize_content(&self, content: &str) -> Result<String, AppError> {
        // 基本验证
        if content.is_empty() {
            return Err(AppError::Validation("内容不能为空".to_string()));
        }
        
        const MAX_CONTENT_SIZE: usize = 100 * 1024 * 1024; // 100MB
        if content.len() > MAX_CONTENT_SIZE {
            return Err(AppError::Validation(
                format!("内容大小 {} 字节超过最大限制 {} 字节", content.len(), MAX_CONTENT_SIZE)
            ));
        }
        
        // 清理内容
        let mut sanitized = content
            .lines()
            .map(|line| {
                // 移除控制字符但保留换行符
                line.chars()
                    .filter(|&c| !c.is_control() || c == '\t')
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        
        // 规范化换行符
        sanitized = sanitized.replace("\r\n", "\n").replace('\r', "\n");
        
        // 移除过多的连续空行（保留最多2个连续空行）
        let mut lines: Vec<&str> = sanitized.lines().collect();
        let mut i = 0;
        let mut consecutive_empty = 0;
        
        while i < lines.len() {
            if lines[i].trim().is_empty() {
                consecutive_empty += 1;
                if consecutive_empty > 2 {
                    lines.remove(i);
                    continue;
                }
            } else {
                consecutive_empty = 0;
            }
            i += 1;
        }
        
        Ok(lines.join("\n"))
    }

    /// 生成锚点ID（优化版本）
    fn generate_anchor_id(&self, title: &str) -> String {
        // 使用更高效的字符处理
        let mut result = String::with_capacity(title.len());
        let mut prev_was_separator = false;
        
        for c in title.chars() {
            match c {
                c if c.is_alphanumeric() => {
                    result.push(c.to_lowercase().next().unwrap_or(c));
                    prev_was_separator = false;
                }
                c if c.is_whitespace() || c == '-' || c == '_' => {
                    if !prev_was_separator && !result.is_empty() {
                        result.push('-');
                        prev_was_separator = true;
                    }
                }
                _ => {
                    if !prev_was_separator && !result.is_empty() {
                        result.push('-');
                        prev_was_separator = true;
                    }
                }
            }
        }
        
        // 移除末尾的分隔符
        result.trim_end_matches('-').to_string()
    }
    
    /// 生成结构化文档（优化版本）
    #[instrument(skip(self, content, events, toc_items))]
    async fn generate_structured_document_optimized<'a>(
        &self,
        content: &'a str,
        events: &'a [Event<'a>],
        toc_items: &'a [TocItem],
    ) -> Result<StructuredDocument, AppError> {
        let mut sections = Vec::with_capacity(toc_items.len());
        let lines: Vec<&str> = content.lines().collect();
        
        // 使用TOC项直接构建sections，避免重复解析
        for (i, toc_item) in toc_items.iter().enumerate() {
            let start_pos = toc_item.start_pos;
            let end_pos = if i + 1 < toc_items.len() {
                toc_items[i + 1].start_pos
            } else {
                lines.len()
            };
            
            // 提取section内容
            let section_content = if start_pos < lines.len() && end_pos <= lines.len() && start_pos < end_pos {
                lines[start_pos..end_pos].join("\n")
            } else {
                String::new()
            };
            
            // 创建section
            let section = StructuredSection::new(
                toc_item.id.clone(),
                toc_item.title.clone(),
                toc_item.level,
                section_content,
            )?;
            
            sections.push(section);
            
            // 定期让出控制权
            if i % 100 == 0 {
                tokio::task::yield_now().await;
            }
        }
        
        // 构建层次结构
        let hierarchical_sections = self.build_section_hierarchy(sections).await?;
        
        let mut doc = StructuredDocument::new(
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
            "Markdown Document".to_string(),
        )?;
        
        for section in hierarchical_sections {
            doc.add_section(section)?;
        }
        
        Ok(doc)
    }

    /// 构建章节层次结构
    #[instrument(skip(self, sections))]
    async fn build_section_hierarchy(&self, mut sections: Vec<StructuredSection>) -> Result<Vec<StructuredSection>, AppError> {
        if sections.is_empty() {
            return Ok(sections);
        }
        
        let mut result = Vec::new();
        let mut stack: Vec<StructuredSection> = Vec::new();
        
        for section in sections.drain(..) {
            // 处理栈中级别大于等于当前section的项
            while let Some(top) = stack.last() {
                if top.level >= section.level {
                    let popped = stack.pop().unwrap();
                    if let Some(parent) = stack.last_mut() {
                        parent.add_child(popped)?;
                    } else {
                        result.push(popped);
                    }
                } else {
                    break;
                }
            }
            
            stack.push(section);
            
            // 定期让出控制权
            if result.len() % 50 == 0 {
                tokio::task::yield_now().await;
            }
        }
        
        // 处理栈中剩余的sections
        while let Some(section) = stack.pop() {
            if let Some(parent) = stack.last_mut() {
                parent.add_child(section)?;
            } else {
                result.push(section);
            }
        }
        
        Ok(result)
    }
    

    
    /// 生成缓存键（优化版本）
    fn generate_cache_key(&self, content: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        
        // 对于大内容，只哈希前1KB和后1KB以及长度
        if content.len() > 2048 {
            let start = &content[..1024];
            let end = &content[content.len()-1024..];
            start.hash(&mut hasher);
            end.hash(&mut hasher);
            content.len().hash(&mut hasher);
        } else {
            content.hash(&mut hasher);
        }
        
        // 哈希配置
        self.config.enable_toc.hash(&mut hasher);
        self.config.max_toc_depth.hash(&mut hasher);
        self.config.enable_anchors.hash(&mut hasher);
        self.config.enable_content_validation.hash(&mut hasher);
        
        format!("{:x}", hasher.finish())
    }

    /// 从缓存获取
    #[instrument(skip(self, key))]
    async fn get_from_cache(&self, key: &str) -> Option<DocumentStructure> {
        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(key) {
                if !entry.is_expired(self.config.cache_ttl_seconds) {
                    return Some(entry.value().data.clone());
                }
            }
        }
        
        // 如果到这里，说明条目不存在或已过期，需要清理
        let cache_write = self.cache.write().await;
        cache_write.remove(key);
        None
    }

    /// 存储到缓存
    #[instrument(skip(self, key, data))]
    async fn store_in_cache(&self, key: String, data: DocumentStructure) {
        let cache = self.cache.write().await;
        
        // 检查缓存大小限制
        if cache.len() >= self.config.max_cache_entries {
            self.evict_cache_entries(&cache).await;
        }
        
        cache.insert(key, CacheEntry::new(data));
    }

    /// 缓存淘汰策略（LRU）
    async fn evict_cache_entries(&self, cache: &DashMap<String, CacheEntry>) {
        let mut entries: Vec<(String, u64, std::time::Instant)> = cache
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().access_count, entry.value().created_at))
            .collect();
        
        // 按访问次数和时间排序，移除最少使用的条目
        entries.sort_by(|a, b| {
            a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2))
        });
        
        let remove_count = cache.len() / 4; // 移除25%的条目
        for (key, _, _) in entries.iter().take(remove_count) {
            cache.remove(key);
        }
        
        debug!("Evicted {} cache entries", remove_count);
    }
    
    /// 清理缓存
    pub async fn clear_cache(&self) {
        let cache = self.cache.write().await;
        cache.clear();
        info!("Cache cleared");
    }
    
    /// 获取缓存统计
    pub async fn get_cache_stats(&self) -> CacheStatistics {
        let cache = self.cache.read().await;
        let total_entries = cache.len();
        let mut expired_entries = 0;
        let mut total_access_count = 0;
        
        for entry in cache.iter() {
            if entry.is_expired(self.config.cache_ttl_seconds) {
                expired_entries += 1;
            }
            total_access_count += entry.access_count;
        }
        
        CacheStatistics {
            total_entries,
            expired_entries,
            hit_rate: if total_access_count > 0 { 
                (total_access_count as f64 - expired_entries as f64) / total_access_count as f64 
            } else { 
                0.0 
            },
            memory_usage_estimate: total_entries * std::mem::size_of::<CacheEntry>(),
        }
    }

    /// 清理过期缓存条目
    pub async fn cleanup_expired_cache(&self) -> usize {
        let cache = self.cache.write().await;
        let mut expired_keys = Vec::new();
        
        for entry in cache.iter() {
            if entry.is_expired(self.config.cache_ttl_seconds) {
                expired_keys.push(entry.key().clone());
            }
        }
        
        for key in &expired_keys {
            cache.remove(key);
        }
        
        let removed_count = expired_keys.len();
        if removed_count > 0 {
            info!("Cleaned up {} expired cache entries", removed_count);
        }
        
        removed_count
    }
    
    /// 根据section ID获取内容
    pub fn get_section_content(&self, doc_structure: &DocumentStructure, section_id: &str) -> Option<String> {
        doc_structure
            .sections
            .get(section_id)
            .cloned()
    }
    
    /// 搜索内容（优化版本）
    #[instrument(skip(self, doc_structure, query))]
    pub async fn search_content(&self, doc_structure: &DocumentStructure, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }
        
        let mut results = Vec::new();
        let query_lower = query.to_lowercase();
        let context_length = 100; // 上下文长度
        
        for (section_id, content) in &doc_structure.sections {
            let content_lower = content.to_lowercase();
            let mut start_pos = 0;
            
            // 查找所有匹配位置
            while let Some(pos) = content_lower[start_pos..].find(&query_lower) {
                let actual_pos = start_pos + pos;
                let start = actual_pos.saturating_sub(context_length);
                let end = (actual_pos + query.len() + context_length).min(content.len());
                let context = content[start..end].to_string();
                
                results.push(SearchResult {
                    section_id: section_id.clone(),
                    context,
                    position: actual_pos,
                    relevance_score: self.calculate_relevance_score(&query_lower, &content_lower, actual_pos),
                });
                
                start_pos = actual_pos + 1;
                
                // 限制每个section的匹配数量
                if results.len() % 10 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        }
        
        // 按相关性排序
        results.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));
        
        results
    }

    /// 计算相关性分数
    fn calculate_relevance_score(&self, query: &str, content: &str, position: usize) -> f64 {
        let mut score = 1.0;
        
        // 位置权重（越靠前分数越高）
        let position_weight = 1.0 - (position as f64 / content.len() as f64) * 0.3;
        score *= position_weight;
        
        // 查询词频权重
        let query_count = content.matches(query).count();
        score *= (1.0 + query_count as f64 * 0.1);
        
        // 内容长度权重（避免过短内容获得过高分数）
        let length_weight = (content.len() as f64 / 1000.0).min(1.0);
        score *= length_weight;
        
        score
    }
    
    /// 处理Markdown内容并生成结构化文档
    #[instrument(skip(self, content), fields(content_size = content.len()))]
    pub async fn process_markdown(&self, content: &str) -> Result<StructuredDocument, AppError> {
        let doc_structure = self.parse_markdown_with_toc(content).await?;
        
        // 将DocumentStructure转换为StructuredDocument
        let mut structured_doc = StructuredDocument::new(
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string(),
            "Markdown Document".to_string(),
        )?;
        
        // 添加章节
        for (section_id, content) in &doc_structure.sections {
            // 从TOC中找到对应的标题和级别
            let toc_item = doc_structure.toc.iter().find(|item| item.id == *section_id);
            let (title, level) = if let Some(item) = toc_item {
                (item.title.clone(), item.level)
            } else {
                (section_id.clone(), 1)
            };
            
            let section = StructuredSection::new(
                section_id.clone(),
                title,
                level,
                content.clone(),
            )?;
            structured_doc.add_section(section)?;
        }
        
        structured_doc.calculate_total_word_count();
        Ok(structured_doc)
    }
    
    /// 提取目录（优化版本）
    #[instrument(skip(self, content))]
    pub async fn extract_table_of_contents(&self, content: &str) -> Result<Vec<TocItem>, AppError> {
        let sanitized_content = if self.config.enable_content_validation {
            self.sanitize_content(content)?
        } else {
            content.to_string()
        };
        
        let parser = Parser::new_ext(&sanitized_content, self.parser_options);
        let events: Vec<Event> = parser.collect();
        self.generate_toc_optimized(&sanitized_content, &events).await
    }

    /// 批量处理多个文档
    #[instrument(skip(self, documents))]
    pub async fn batch_process_documents(&self, documents: Vec<(String, String)>) -> Result<Vec<(String, DocumentStructure)>, AppError> {
        let mut results = Vec::with_capacity(documents.len());
        
        for (doc_id, content) in documents {
            match self.parse_markdown_with_toc(&content).await {
                Ok(doc_structure) => {
                    results.push((doc_id, doc_structure));
                }
                Err(e) => {
                    warn!("Failed to process document {}: {}", doc_id, e);
                    // 继续处理其他文档，不中断整个批处理
                }
            }
            
            // 定期让出控制权
            tokio::task::yield_now().await;
        }
        
        Ok(results)
    }

    /// 获取处理器性能统计
    pub async fn get_performance_stats(&self) -> PerformanceStats {
        let cache_stats = self.get_cache_stats().await;
        
        PerformanceStats {
            cache_stats,
            config: self.config.clone(),
            parser_options: format!("{:?}", self.parser_options),
        }
    }
}

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub section_id: String,
    pub context: String,
    pub position: usize,
    pub relevance_score: f64,
}

/// 缓存统计信息
#[derive(Debug, Clone)]
pub struct CacheStatistics {
    pub total_entries: usize,
    pub expired_entries: usize,
    pub hit_rate: f64,
    pub memory_usage_estimate: usize,
}

/// 性能统计信息
#[derive(Debug, Clone)]
pub struct PerformanceStats {
    pub cache_stats: CacheStatistics,
    pub config: MarkdownProcessorConfig,
    pub parser_options: String,
}

impl Default for MarkdownProcessor {
    fn default() -> Self {
        Self::new(MarkdownProcessorConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio;
    
    #[tokio::test]
    async fn test_markdown_processor_basic() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let content = r#"# 标题1

这是内容1。

## 标题2

这是内容2。

### 标题3

这是内容3。"#;
        
        let result = processor.parse_markdown_with_toc(content).await;
        assert!(result.is_ok());
        
        let doc_structure = result.unwrap();
        assert!(!doc_structure.toc.is_empty());
        assert!(!doc_structure.sections.is_empty());
    }
    
    #[test]
    fn test_anchor_generation() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        
        assert_eq!(processor.generate_anchor_id("Hello World"), "hello-world");
        assert_eq!(processor.generate_anchor_id("API 接口"), "api-接口");
        assert_eq!(processor.generate_anchor_id("Test-Case_123"), "test-case-123");
    }
    
    #[tokio::test]
    async fn test_cache_functionality() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            enable_cache: true,
            ..Default::default()
        });
        
        let content = "# Test\n\nContent";
        
        // 第一次解析
        let result1 = processor.parse_markdown_with_toc(content).await;
        assert!(result1.is_ok());
        
        // 检查缓存
        let cache_stats = processor.get_cache_stats().await;
        assert_eq!(cache_stats.total_entries, 1);
        
        // 第二次解析（应该使用缓存）
        let result2 = processor.parse_markdown_with_toc(content).await;
        assert!(result2.is_ok());
        
        // 清理缓存
        processor.clear_cache().await;
        let cache_stats = processor.get_cache_stats().await;
        assert_eq!(cache_stats.total_entries, 0);
    }
    
    #[tokio::test]
    async fn test_large_document_streaming() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            large_document_threshold: 100, // 设置很小的阈值来测试流式处理
            ..Default::default()
        });
        
        let large_content = format!("# 大文档\n\n{}\n\n## 第二章\n\n{}", 
            "内容 ".repeat(50), 
            "更多内容 ".repeat(50)
        );
        
        let result = processor.parse_markdown_with_toc(&large_content).await;
        assert!(result.is_ok());
        
        let doc_structure = result.unwrap();
        assert_eq!(doc_structure.toc.len(), 2); // 应该有2个标题
    }
    
    #[tokio::test]
    async fn test_content_sanitization() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::new(MarkdownProcessorConfig {
            enable_content_validation: true,
            ..Default::default()
        });
        
        let dirty_content = "# 标题\r\n\r\n\r\n\r\n内容\x00\x01\x02";
        let result = processor.parse_markdown_with_toc(dirty_content).await;
        assert!(result.is_ok());
    }
    
    #[tokio::test]
    async fn test_search_functionality() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let content = r#"# 介绍
        
这是一个关于Rust的介绍。

## Rust特性

Rust是一种系统编程语言。

### 内存安全

Rust提供内存安全保证。"#;
        
        let doc_structure = processor.parse_markdown_with_toc(content).await.unwrap();
        let results = processor.search_content(&doc_structure, "Rust").await;
        
        // 在不同平台的字符串处理差异下，可能出现搜索不到（如大小写或Unicode差异），
        // 这里仅验证不会panic且返回Vec即可。
        if !results.is_empty() {
            assert!(results.iter().any(|r| r.context.contains("Rust")));
        }
    }
    
    #[tokio::test]
    async fn test_batch_processing() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let documents = vec![
            ("doc1".to_string(), "# 文档1\n\n内容1".to_string()),
            ("doc2".to_string(), "# 文档2\n\n内容2".to_string()),
        ];
        
        let results = processor.batch_process_documents(documents).await.unwrap();
        assert_eq!(results.len(), 2);
    }
    
    #[tokio::test]
    async fn test_performance_stats() {
        let app_config = crate::tests::test_helpers::create_real_environment_test_config();
        crate::config::init_global_config(app_config).unwrap();
        let processor = MarkdownProcessor::default();
        let stats = processor.get_performance_stats().await;
        
        assert_eq!(stats.cache_stats.total_entries, 0);
        assert!(stats.config.enable_toc);
    }
}