use crate::error::AppError;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// 统一结构化文档
///
/// 表示一个完整的结构化文档，包含文档的基本信息、目录结构、性能指标等。
/// 支持快速查找和索引功能，适用于大型文档的高效处理。
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct StructuredDocument {
    /// 文档处理任务的唯一标识符
    pub task_id: String,

    /// 文档的标题或名称
    pub document_title: String,

    /// 文档的目录结构，包含所有章节和子章节
    pub toc: Vec<StructuredSection>,

    /// 文档中章节的总数量
    pub total_sections: usize,

    /// 文档最后更新的时间戳（UTC时区）
    pub last_updated: DateTime<Utc>,

    /// 文档的总字数统计（可选）
    pub word_count: Option<usize>,

    /// 文档处理所需的时间（可选，格式如 "2.5s"）
    pub processing_time: Option<String>,

    // Performance optimization fields
    /// 章节ID到索引位置的映射，用于O(1)时间复杂度的快速查找
    /// 序列化时跳过此字段
    #[serde(skip)]
    section_index: HashMap<String, usize>, // ID -> index mapping for O(1) lookup

    /// 章节层级到索引列表的映射，用于按层级快速检索
    /// 序列化时跳过此字段
    #[serde(skip)]
    level_index: HashMap<u8, Vec<usize>>, // Level -> indices mapping

    /// 标记索引是否已构建，避免重复构建索引
    /// 序列化时跳过此字段
    #[serde(skip)]
    is_indexed: bool, // Track if indices are built
}

/// 结构化章节
///
/// 表示文档中的一个章节或段落，支持嵌套的层级结构。
/// 包含内容、元数据和性能优化字段，适用于大型内容的处理。
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct StructuredSection {
    /// 章节的唯一标识符，通常基于标题生成
    pub id: String,

    /// 章节的标题或名称
    pub title: String,

    /// 章节的层级深度，1表示顶级章节，2表示二级章节，以此类推
    pub level: u8,

    /// 章节的正文内容
    pub content: String,

    /// 子章节列表，支持无限层级的嵌套结构
    /// 当为空时序列化时会被跳过，避免空数组的序列化
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[schema(no_recursion)]
    pub children: Vec<Box<StructuredSection>>,

    /// 标记章节是否已被编辑过（可选）
    pub is_edited: Option<bool>,

    /// 章节的字数统计（可选）
    pub word_count: Option<usize>,

    /// 章节在原文中的起始位置（可选，用于定位和引用）
    pub start_pos: Option<usize>,

    /// 章节在原文中的结束位置（可选，用于定位和引用）
    pub end_pos: Option<usize>,

    // Performance optimization fields
    /// 内容的哈希值，用于检测内容变化，避免不必要的重新处理
    /// 序列化时跳过此字段
    #[serde(skip)]
    content_hash: Option<u64>, // For change detection

    /// 标记内容是否超过阈值，用于性能优化策略
    /// 序列化时跳过此字段
    #[serde(skip)]
    is_large_content: bool, // Flag for content > threshold
}

impl StructuredSection {
    /// Content size threshold for large content optimization (50KB)
    const LARGE_CONTENT_THRESHOLD: usize = 50 * 1024;

    /// 创建新的章节
    pub fn new(id: String, title: String, level: u8, content: String) -> Result<Self, AppError> {
        // Validate inputs
        if id.is_empty() {
            return Err(AppError::Validation("章节ID不能为空".to_string()));
        }

        if title.is_empty() {
            return Err(AppError::Validation("章节标题不能为空".to_string()));
        }

        if level == 0 || level > 6 {
            return Err(AppError::Validation("章节级别必须在1-6之间".to_string()));
        }

        let word_count = Self::calculate_word_count(&content);
        let content_hash = Self::calculate_content_hash(&content);
        let is_large_content = content.len() > Self::LARGE_CONTENT_THRESHOLD;

        Ok(Self {
            id,
            title,
            level,
            content,
            children: Vec::new(),
            is_edited: Some(false),
            word_count: Some(word_count),
            start_pos: None,
            end_pos: None,
            content_hash: Some(content_hash),
            is_large_content,
        })
    }

    /// 创建新的章节（不验证，用于内部使用）
    pub fn new_unchecked(id: String, title: String, level: u8, content: String) -> Self {
        let word_count = Self::calculate_word_count(&content);
        let content_hash = Self::calculate_content_hash(&content);
        let is_large_content = content.len() > Self::LARGE_CONTENT_THRESHOLD;

        Self {
            id,
            title,
            level,
            content,
            children: Vec::new(),
            is_edited: Some(false),
            word_count: Some(word_count),
            start_pos: None,
            end_pos: None,
            content_hash: Some(content_hash),
            is_large_content,
        }
    }

    /// 计算内容哈希值（用于变更检测）
    fn calculate_content_hash(content: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// 计算字数（优化版本）
    fn calculate_word_count(content: &str) -> usize {
        if content.is_empty() {
            return 0;
        }

        // For large content, use approximate counting for performance
        if content.len() > Self::LARGE_CONTENT_THRESHOLD {
            // Approximate: assume average word length of 5 characters
            content.len() / 5
        } else {
            content.split_whitespace().count()
        }
    }

    /// 验证章节数据
    pub fn validate(&self) -> Result<(), AppError> {
        if self.id.is_empty() {
            return Err(AppError::Validation("章节ID不能为空".to_string()));
        }

        if self.title.is_empty() {
            return Err(AppError::Validation("章节标题不能为空".to_string()));
        }

        if self.level == 0 || self.level > 6 {
            return Err(AppError::Validation("章节级别必须在1-6之间".to_string()));
        }

        // Validate content size
        const MAX_CONTENT_SIZE: usize = 10 * 1024 * 1024; // 10MB
        if self.content.len() > MAX_CONTENT_SIZE {
            return Err(AppError::Validation(format!(
                "章节内容大小 {} 字节超过最大限制 {} 字节",
                self.content.len(),
                MAX_CONTENT_SIZE
            )));
        }

        // Validate children recursively
        for child in &self.children {
            child.as_ref().validate()?;
        }

        Ok(())
    }

    /// 添加子章节（带验证）
    pub fn add_child(&mut self, child: StructuredSection) -> Result<(), AppError> {
        // Validate child level is appropriate
        if child.level <= self.level {
            return Err(AppError::Validation(format!(
                "子章节级别 {} 必须大于父章节级别 {}",
                child.level, self.level
            )));
        }

        // Validate child
        child.validate()?;

        // Check for duplicate IDs
        if self.find_child_by_id(&child.id).is_some() {
            return Err(AppError::Validation(format!(
                "子章节ID {} 已存在",
                child.id
            )));
        }

        self.children.push(Box::new(child));
        Ok(())
    }

    /// 查找直接子章节
    pub fn find_child_by_id(&self, id: &str) -> Option<&StructuredSection> {
        self.children
            .iter()
            .find(|child| child.id == id)
            .map(|boxed| boxed.as_ref())
    }

    /// 查找直接子章节（可变引用）
    pub fn find_child_by_id_mut(&mut self, id: &str) -> Option<&mut StructuredSection> {
        self.children
            .iter_mut()
            .find(|child| child.id == id)
            .map(|boxed| boxed.as_mut())
    }

    /// 获取所有子章节（包括嵌套的）- 优化版本
    pub fn get_all_children(&self) -> Vec<&StructuredSection> {
        let mut all_children = Vec::with_capacity(self.estimate_total_children());
        self.collect_all_children(&mut all_children);
        all_children
    }

    /// 估算总子章节数量（用于预分配）
    fn estimate_total_children(&self) -> usize {
        let mut count = self.children.len();
        for child in &self.children {
            count += child.estimate_total_children();
        }
        count
    }

    /// 收集所有子章节（递归）
    fn collect_all_children<'a>(&'a self, result: &mut Vec<&'a StructuredSection>) {
        for child in &self.children {
            result.push(child.as_ref());
            child.collect_all_children(result);
        }
    }

    /// 获取章节深度
    pub fn get_depth(&self) -> usize {
        if self.children.is_empty() {
            1
        } else {
            1 + self
                .children
                .iter()
                .map(|c| c.get_depth())
                .max()
                .unwrap_or(1)
        }
    }

    /// 检查是否有子章节
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    /// 获取章节路径（用于导航）
    pub fn get_path(&self) -> String {
        if self.level == 1 {
            self.title.clone()
        } else {
            format!(
                "{} > {}",
                "  ".repeat((self.level - 1) as usize),
                self.title
            )
        }
    }

    /// 更新内容（带变更检测）
    pub fn update_content(&mut self, new_content: String) -> Result<bool, AppError> {
        let new_hash = Self::calculate_content_hash(&new_content);
        let content_changed = self.content_hash != Some(new_hash);

        if content_changed {
            // Validate new content size
            const MAX_CONTENT_SIZE: usize = 10 * 1024 * 1024; // 10MB
            if new_content.len() > MAX_CONTENT_SIZE {
                return Err(AppError::Validation(format!(
                    "内容大小 {} 字节超过最大限制 {} 字节",
                    new_content.len(),
                    MAX_CONTENT_SIZE
                )));
            }

            self.content = new_content;
            self.content_hash = Some(new_hash);
            self.word_count = Some(Self::calculate_word_count(&self.content));
            self.is_large_content = self.content.len() > Self::LARGE_CONTENT_THRESHOLD;
            self.is_edited = Some(true);
        }

        Ok(content_changed)
    }

    /// 获取内容摘要（用于大内容预览）
    pub fn get_content_summary(&self, max_length: usize) -> String {
        if self.content.len() <= max_length {
            self.content.clone()
        } else {
            let truncated = &self.content[..max_length];
            format!("{truncated}...")
        }
    }

    /// 检查内容是否已更改
    pub fn is_content_changed(&self) -> bool {
        self.is_edited.unwrap_or(false)
    }

    /// 获取内容大小（字节）
    pub fn get_content_size(&self) -> usize {
        self.content.len()
    }

    /// 检查是否为大内容
    pub fn is_large_content(&self) -> bool {
        self.is_large_content
    }

    /// 清理内容（移除多余空白）
    pub fn sanitize_content(&mut self) -> Result<(), AppError> {
        let sanitized = self
            .content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        self.update_content(sanitized)?;
        Ok(())
    }
}

impl StructuredDocument {
    /// 创建新的结构化文档
    pub fn new(task_id: String, document_title: String) -> Result<Self, AppError> {
        // Validate inputs
        if task_id.is_empty() {
            return Err(AppError::Validation("任务ID不能为空".to_string()));
        }

        if document_title.is_empty() {
            return Err(AppError::Validation("文档标题不能为空".to_string()));
        }

        Ok(Self {
            task_id,
            document_title,
            toc: Vec::new(),
            total_sections: 0,
            last_updated: Utc::now(),
            word_count: None,
            processing_time: None,
            section_index: HashMap::new(),
            level_index: HashMap::new(),
            is_indexed: false,
        })
    }

    /// 创建新的结构化文档（不验证，用于内部使用）
    pub fn new_unchecked(task_id: String, document_title: String) -> Self {
        Self {
            task_id,
            document_title,
            toc: Vec::new(),
            total_sections: 0,
            last_updated: Utc::now(),
            word_count: None,
            processing_time: None,
            section_index: HashMap::new(),
            level_index: HashMap::new(),
            is_indexed: false,
        }
    }

    /// 验证文档数据
    pub fn validate(&self) -> Result<(), AppError> {
        if self.task_id.is_empty() {
            return Err(AppError::Validation("任务ID不能为空".to_string()));
        }

        if self.document_title.is_empty() {
            return Err(AppError::Validation("文档标题不能为空".to_string()));
        }

        // Validate sections
        for section in &self.toc {
            section.validate()?;
        }

        // Check for duplicate section IDs
        let mut seen_ids = std::collections::HashSet::new();
        self.collect_all_section_ids(&self.toc, &mut seen_ids)?;

        Ok(())
    }

    /// 收集所有章节ID并检查重复
    fn collect_all_section_ids(
        &self,
        sections: &[StructuredSection],
        seen_ids: &mut std::collections::HashSet<String>,
    ) -> Result<(), AppError> {
        for section in sections {
            if !seen_ids.insert(section.id.clone()) {
                return Err(AppError::Validation(format!(
                    "重复的章节ID: {}",
                    section.id
                )));
            }
            // Convert Vec<Box<StructuredSection>> to slice for recursion
            let children_slice: Vec<&StructuredSection> = section
                .children
                .iter()
                .map(|boxed| boxed.as_ref())
                .collect();
            for child in &children_slice {
                self.collect_all_section_ids(&[(*child).clone()], seen_ids)?;
            }
        }
        Ok(())
    }

    /// 构建索引（用于快速查找）
    pub fn build_index(&mut self) {
        self.section_index.clear();
        self.level_index.clear();

        // Clone toc to avoid borrowing issues
        let toc_clone = self.toc.clone();
        self.build_section_index(&toc_clone, 0);
        self.is_indexed = true;
    }

    /// 递归构建章节索引
    fn build_section_index(&mut self, sections: &[StructuredSection], base_index: usize) {
        for (i, section) in sections.iter().enumerate() {
            let section_index = base_index + i;

            // Build ID index
            self.section_index.insert(section.id.clone(), section_index);

            // Build level index
            self.level_index
                .entry(section.level)
                .or_default()
                .push(section_index);
        }

        // Recursively index children in a separate loop to avoid borrowing issues
        for (i, section) in sections.iter().enumerate() {
            let section_index = base_index + i;
            if !section.children.is_empty() {
                // Convert Vec<Box<StructuredSection>> to slice for recursion
                let children_slice: Vec<&StructuredSection> = section
                    .children
                    .iter()
                    .map(|boxed| boxed.as_ref())
                    .collect();
                let children_owned: Vec<StructuredSection> =
                    children_slice.iter().map(|&child| child.clone()).collect();
                self.build_section_index(&children_owned, section_index * 1000);
            }
        }
    }

    /// 确保索引已构建
    fn ensure_indexed(&mut self) {
        if !self.is_indexed {
            self.build_index();
        }
    }

    /// 添加章节（带验证和索引更新）
    pub fn add_section(&mut self, section: StructuredSection) -> Result<(), AppError> {
        // Validate section
        section.validate()?;

        // Check for duplicate ID
        if self.find_section_by_id(&section.id).is_some() {
            return Err(AppError::Validation(format!(
                "章节ID {} 已存在",
                section.id
            )));
        }

        self.toc.push(section);
        self.total_sections = self.toc.len();
        self.last_updated = Utc::now();

        // Mark index as dirty
        self.is_indexed = false;

        Ok(())
    }

    /// 批量添加章节（性能优化）
    pub fn add_sections(&mut self, sections: Vec<StructuredSection>) -> Result<(), AppError> {
        // Validate all sections first
        for section in &sections {
            section.validate()?;
        }

        // Check for duplicate IDs
        let mut existing_ids = std::collections::HashSet::new();
        self.collect_all_section_ids(&self.toc, &mut existing_ids)?;

        for section in &sections {
            if existing_ids.contains(&section.id) {
                return Err(AppError::Validation(format!(
                    "章节ID {} 已存在",
                    section.id
                )));
            }
            existing_ids.insert(section.id.clone());
        }

        // Add all sections
        self.toc.extend(sections);
        self.total_sections = self.toc.len();
        self.last_updated = Utc::now();

        // Mark index as dirty
        self.is_indexed = false;

        Ok(())
    }

    /// 计算总字数
    pub fn calculate_total_word_count(&mut self) {
        let total = self
            .toc
            .iter()
            .map(|section| self.calculate_section_word_count(section))
            .sum::<usize>();
        self.word_count = Some(total);
    }

    /// 递归计算章节字数
    fn calculate_section_word_count(&self, section: &StructuredSection) -> usize {
        let section_count = section.word_count.unwrap_or(0);
        let children_count = section
            .children
            .iter()
            .map(|child| self.calculate_section_word_count(child.as_ref()))
            .sum::<usize>();
        section_count + children_count
    }

    /// 获取指定级别的章节（优化版本）
    pub fn get_sections_by_level(&mut self, level: u8) -> Vec<&StructuredSection> {
        self.ensure_indexed();

        if let Some(indices) = self.level_index.get(&level) {
            indices
                .iter()
                .filter_map(|&index| self.get_section_by_index(index))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// 根据索引获取章节
    fn get_section_by_index(&self, index: usize) -> Option<&StructuredSection> {
        if index < 1000 {
            // Top-level section
            self.toc.get(index)
        } else {
            // Child section - decode index
            let parent_index = index / 1000;
            let child_index = index % 1000;
            self.toc
                .get(parent_index)?
                .children
                .get(child_index)
                .map(|boxed| boxed.as_ref())
        }
    }

    /// 根据ID查找章节（优化版本）
    pub fn find_section_by_id(&self, id: &str) -> Option<&StructuredSection> {
        if self.is_indexed {
            // Use index for O(1) lookup
            if let Some(&index) = self.section_index.get(id) {
                self.get_section_by_index(index)
            } else {
                None
            }
        } else {
            // Fallback to recursive search
            self.find_section_recursive(&self.toc, id)
        }
    }

    /// 根据ID查找章节（可变引用）
    pub fn find_section_by_id_mut(&mut self, id: &str) -> Option<&mut StructuredSection> {
        Self::find_section_recursive_mut(&mut self.toc, id)
    }

    /// 递归查找章节（可变引用）
    fn find_section_recursive_mut<'a>(
        sections: &'a mut [StructuredSection],
        target_id: &str,
    ) -> Option<&'a mut StructuredSection> {
        for section in sections {
            if section.id == target_id {
                return Some(section);
            }
            // Recursively search in children
            for child in &mut section.children {
                if let Some(found) = Self::find_section_recursive_mut(
                    std::slice::from_mut(child.as_mut()),
                    target_id,
                ) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// 递归查找章节（不可变引用）
    fn find_section_recursive<'a>(
        &'a self,
        sections: &'a [StructuredSection],
        target_id: &str,
    ) -> Option<&'a StructuredSection> {
        for section in sections {
            if section.id == target_id {
                return Some(section);
            }
            // Recursively search in children
            for child in &section.children {
                if let Some(found) =
                    self.find_section_recursive(std::slice::from_ref(child.as_ref()), target_id)
                {
                    return Some(found);
                }
            }
        }
        None
    }

    /// 获取所有章节（扁平化）
    pub fn get_all_sections(&self) -> Vec<&StructuredSection> {
        let mut all_sections = Vec::with_capacity(self.estimate_total_sections());
        self.collect_all_sections(&self.toc, &mut all_sections);
        all_sections
    }

    /// 估算总章节数量
    fn estimate_total_sections(&self) -> usize {
        let mut count = self.toc.len();
        for section in &self.toc {
            count += section.estimate_total_children();
        }
        count
    }

    /// 收集所有章节
    fn collect_all_sections<'a>(
        &'a self,
        sections: &'a [StructuredSection],
        result: &mut Vec<&'a StructuredSection>,
    ) {
        for section in sections {
            result.push(section);
            for child in &section.children {
                self.collect_all_sections(std::slice::from_ref(child.as_ref()), result);
            }
        }
    }

    /// 更新章节内容
    pub fn update_section_content(
        &mut self,
        section_id: &str,
        new_content: String,
    ) -> Result<bool, AppError> {
        if let Some(section) = self.find_section_by_id_mut(section_id) {
            let changed = section.update_content(new_content)?;
            if changed {
                self.last_updated = Utc::now();
            }
            Ok(changed)
        } else {
            Err(AppError::Validation(format!("未找到章节ID: {section_id}")))
        }
    }

    /// 删除章节
    pub fn remove_section(&mut self, section_id: &str) -> Result<bool, AppError> {
        if Self::remove_section_recursive(&mut self.toc, section_id) {
            self.total_sections = self.toc.len();
            self.last_updated = Utc::now();
            self.is_indexed = false; // Mark index as dirty
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 递归删除章节
    fn remove_section_recursive(sections: &mut Vec<StructuredSection>, target_id: &str) -> bool {
        for i in 0..sections.len() {
            if sections[i].id == target_id {
                sections.remove(i);
                return true;
            }
            // Convert Vec<Box<StructuredSection>> to Vec<StructuredSection> for recursion
            let mut children_vec: Vec<StructuredSection> =
                sections[i].children.drain(..).map(|boxed| *boxed).collect();
            if Self::remove_section_recursive(&mut children_vec, target_id) {
                sections[i].children = children_vec.into_iter().map(Box::new).collect();
                return true;
            }
            sections[i].children = children_vec.into_iter().map(Box::new).collect();
        }
        false
    }

    /// 获取文档统计信息
    pub fn get_statistics(&mut self) -> DocumentStatistics {
        self.calculate_total_word_count();

        let all_sections = self.get_all_sections();
        let large_sections = all_sections.iter().filter(|s| s.is_large_content()).count();

        let max_depth = self.toc.iter().map(|s| s.get_depth()).max().unwrap_or(0);

        DocumentStatistics {
            total_sections: all_sections.len(),
            total_word_count: self.word_count.unwrap_or(0),
            max_depth,
            large_sections_count: large_sections,
            last_updated: self.last_updated,
        }
    }

    /// 清理文档（移除空章节，整理内容）
    pub fn cleanup(&mut self) -> Result<usize, AppError> {
        let mut removed_count = 0;

        // Remove empty sections and sanitize content
        Self::cleanup_sections(&mut self.toc, &mut removed_count)?;

        if removed_count > 0 {
            self.total_sections = self.toc.len();
            self.last_updated = Utc::now();
            self.is_indexed = false;
        }

        Ok(removed_count)
    }

    /// 递归清理章节
    fn cleanup_sections(
        sections: &mut Vec<StructuredSection>,
        removed_count: &mut usize,
    ) -> Result<(), AppError> {
        let mut i = 0;
        while i < sections.len() {
            let section = &mut sections[i];

            // Sanitize content
            section.sanitize_content()?;

            // Recursively cleanup children
            let mut children_vec: Vec<StructuredSection> =
                section.children.drain(..).map(|boxed| *boxed).collect();
            Self::cleanup_sections(&mut children_vec, removed_count)?;
            section.children = children_vec.into_iter().map(Box::new).collect();

            // Remove if empty after cleanup
            if section.content.trim().is_empty() && section.children.is_empty() {
                sections.remove(i);
                *removed_count += 1;
            } else {
                i += 1;
            }
        }
        Ok(())
    }

    /// 获取内存使用情况
    pub fn get_memory_usage(&self) -> MemoryUsage {
        let mut total_content_size = 0;
        let mut section_count = 0;

        self.calculate_memory_usage(&self.toc, &mut total_content_size, &mut section_count);

        let index_size = self.section_index.len()
            * (std::mem::size_of::<String>() + std::mem::size_of::<usize>())
            + self.level_index.len()
                * (std::mem::size_of::<u8>() + std::mem::size_of::<Vec<usize>>());

        MemoryUsage {
            total_content_size,
            section_count,
            index_size,
            estimated_total_size: total_content_size
                + index_size
                + section_count * std::mem::size_of::<StructuredSection>(),
        }
    }

    /// 递归计算内存使用
    fn calculate_memory_usage(
        &self,
        sections: &[StructuredSection],
        content_size: &mut usize,
        count: &mut usize,
    ) {
        for section in sections {
            *content_size += section.content.len();
            *count += 1;
            // Convert Vec<Box<StructuredSection>> to slice for recursion
            let children_slice: Vec<&StructuredSection> = section
                .children
                .iter()
                .map(|boxed| boxed.as_ref())
                .collect();
            let children_owned: Vec<StructuredSection> =
                children_slice.iter().map(|&child| child.clone()).collect();
            self.calculate_memory_usage(&children_owned, content_size, count);
        }
    }
}

/// 文档统计信息
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DocumentStatistics {
    pub total_sections: usize,
    pub total_word_count: usize,
    pub max_depth: usize,
    pub large_sections_count: usize,
    pub last_updated: DateTime<Utc>,
}

/// 内存使用情况
#[derive(Debug, Clone)]
pub struct MemoryUsage {
    pub total_content_size: usize,
    pub section_count: usize,
    pub index_size: usize,
    pub estimated_total_size: usize,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_section_creation() {
        let section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "This is test content.".to_string(),
        )
        .unwrap();

        assert_eq!(section.id, "section1");
        assert_eq!(section.title, "Test Section");
        assert_eq!(section.level, 1);
        assert_eq!(section.content, "This is test content.");
        assert_eq!(section.word_count, Some(4));
        assert!(!section.is_large_content());
        assert!(section.content_hash.is_some());
    }

    #[test]
    fn test_structured_section_validation() {
        // Empty ID should fail
        assert!(
            StructuredSection::new("".to_string(), "Test".to_string(), 1, "Content".to_string(),)
                .is_err()
        );

        // Empty title should fail
        assert!(
            StructuredSection::new("id1".to_string(), "".to_string(), 1, "Content".to_string(),)
                .is_err()
        );

        // Invalid level should fail
        assert!(
            StructuredSection::new(
                "id1".to_string(),
                "Test".to_string(),
                0,
                "Content".to_string(),
            )
            .is_err()
        );

        assert!(
            StructuredSection::new(
                "id1".to_string(),
                "Test".to_string(),
                7,
                "Content".to_string(),
            )
            .is_err()
        );
    }

    #[test]
    fn test_structured_section_large_content() {
        let large_content = "x".repeat(60 * 1024); // 60KB
        let section = StructuredSection::new(
            "large1".to_string(),
            "Large Section".to_string(),
            1,
            large_content,
        )
        .unwrap();

        assert!(section.is_large_content());
        // Word count should be approximate for large content
        assert!(section.word_count.unwrap() > 0);
    }

    #[test]
    fn test_structured_section_add_child() {
        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let child = StructuredSection::new(
            "child".to_string(),
            "Child Section".to_string(),
            2,
            "Child content".to_string(),
        )
        .unwrap();

        assert!(parent.add_child(child).is_ok());
        assert!(parent.has_children());
        assert_eq!(parent.children.len(), 1);
    }

    #[test]
    fn test_structured_section_add_child_validation() {
        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            2,
            "Parent content".to_string(),
        )
        .unwrap();

        // Child with same or lower level should fail
        let invalid_child = StructuredSection::new(
            "child".to_string(),
            "Child Section".to_string(),
            2,
            "Child content".to_string(),
        )
        .unwrap();

        assert!(parent.add_child(invalid_child).is_err());
    }

    #[test]
    fn test_structured_section_update_content() {
        let mut section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "Original content".to_string(),
        )
        .unwrap();

        let original_hash = section.content_hash;

        // Update with new content
        let changed = section.update_content("New content".to_string()).unwrap();
        assert!(changed);
        assert_ne!(section.content_hash, original_hash);
        assert_eq!(section.content, "New content");
        assert_eq!(section.is_edited, Some(true));

        // Update with same content
        let changed = section.update_content("New content".to_string()).unwrap();
        assert!(!changed);
    }

    #[test]
    fn test_structured_section_content_summary() {
        let section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "This is a very long content that should be truncated".to_string(),
        )
        .unwrap();

        let summary = section.get_content_summary(20);
        assert_eq!(summary, "This is a very long ...");

        let full_summary = section.get_content_summary(100);
        assert_eq!(full_summary, section.content);
    }

    #[test]
    fn test_structured_section_sanitize_content() {
        let mut section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "  Line 1  \n\n  Line 2  \n\n\n  Line 3  \n".to_string(),
        )
        .unwrap();

        section.sanitize_content().unwrap();
        assert_eq!(section.content, "Line 1\nLine 2\nLine 3");
    }

    #[test]
    fn test_structured_document_creation() {
        let doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        assert_eq!(doc.task_id, "task1");
        assert_eq!(doc.document_title, "Test Document");
        assert_eq!(doc.total_sections, 0);
        assert!(doc.toc.is_empty());
        assert!(!doc.is_indexed);
    }

    #[test]
    fn test_structured_document_validation() {
        // Empty task ID should fail
        assert!(StructuredDocument::new("".to_string(), "Test Document".to_string(),).is_err());

        // Empty title should fail
        assert!(StructuredDocument::new("task1".to_string(), "".to_string(),).is_err());
    }

    #[test]
    fn test_structured_document_add_section() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "Test content".to_string(),
        )
        .unwrap();

        assert!(doc.add_section(section).is_ok());
        assert_eq!(doc.total_sections, 1);
        assert!(!doc.is_indexed); // Should mark index as dirty
    }

    #[test]
    fn test_structured_document_add_duplicate_section() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section1 = StructuredSection::new(
            "section1".to_string(),
            "Test Section 1".to_string(),
            1,
            "Test content 1".to_string(),
        )
        .unwrap();

        let section2 = StructuredSection::new(
            "section1".to_string(), // Same ID
            "Test Section 2".to_string(),
            1,
            "Test content 2".to_string(),
        )
        .unwrap();

        assert!(doc.add_section(section1).is_ok());
        assert!(doc.add_section(section2).is_err()); // Should fail due to duplicate ID
    }

    #[test]
    fn test_structured_document_batch_add_sections() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let sections = vec![
            StructuredSection::new(
                "section1".to_string(),
                "Section 1".to_string(),
                1,
                "Content 1".to_string(),
            )
            .unwrap(),
            StructuredSection::new(
                "section2".to_string(),
                "Section 2".to_string(),
                1,
                "Content 2".to_string(),
            )
            .unwrap(),
        ];

        assert!(doc.add_sections(sections).is_ok());
        assert_eq!(doc.total_sections, 2);
    }

    #[test]
    fn test_structured_document_indexing() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section1 = StructuredSection::new(
            "section1".to_string(),
            "Section 1".to_string(),
            1,
            "Content 1".to_string(),
        )
        .unwrap();

        let section2 = StructuredSection::new(
            "section2".to_string(),
            "Section 2".to_string(),
            2,
            "Content 2".to_string(),
        )
        .unwrap();

        doc.add_section(section1).unwrap();
        doc.add_section(section2).unwrap();

        // Build index
        doc.build_index();
        assert!(doc.is_indexed);

        // Test indexed lookup
        let found = doc.find_section_by_id("section1");
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "section1");

        // Test level-based lookup
        let level1_sections = doc.get_sections_by_level(1);
        assert_eq!(level1_sections.len(), 1);
        assert_eq!(level1_sections[0].id, "section1");

        let level2_sections = doc.get_sections_by_level(2);
        assert_eq!(level2_sections.len(), 1);
        assert_eq!(level2_sections[0].id, "section2");
    }

    #[test]
    fn test_structured_document_find_section() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let child = StructuredSection::new(
            "child".to_string(),
            "Child Section".to_string(),
            2,
            "Child content".to_string(),
        )
        .unwrap();

        parent.add_child(child).unwrap();
        doc.add_section(parent).unwrap();

        // Test finding parent
        let found_parent = doc.find_section_by_id("parent");
        assert!(found_parent.is_some());
        assert_eq!(found_parent.unwrap().id, "parent");

        // Test finding child
        let found_child = doc.find_section_by_id("child");
        assert!(found_child.is_some());
        assert_eq!(found_child.unwrap().id, "child");

        // Test not found
        let not_found = doc.find_section_by_id("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_structured_document_update_section_content() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "Original content".to_string(),
        )
        .unwrap();

        doc.add_section(section).unwrap();

        // Update existing section
        let changed = doc
            .update_section_content("section1", "New content".to_string())
            .unwrap();
        assert!(changed);

        let updated_section = doc.find_section_by_id("section1").unwrap();
        assert_eq!(updated_section.content, "New content");

        // Try to update non-existent section
        assert!(
            doc.update_section_content("nonexistent", "Content".to_string())
                .is_err()
        );
    }

    #[test]
    fn test_structured_document_remove_section() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section1 = StructuredSection::new(
            "section1".to_string(),
            "Section 1".to_string(),
            1,
            "Content 1".to_string(),
        )
        .unwrap();

        let section2 = StructuredSection::new(
            "section2".to_string(),
            "Section 2".to_string(),
            1,
            "Content 2".to_string(),
        )
        .unwrap();

        doc.add_section(section1).unwrap();
        doc.add_section(section2).unwrap();
        assert_eq!(doc.total_sections, 2);

        // Remove existing section
        let removed = doc.remove_section("section1").unwrap();
        assert!(removed);
        assert_eq!(doc.total_sections, 1);
        assert!(doc.find_section_by_id("section1").is_none());

        // Try to remove non-existent section
        let not_removed = doc.remove_section("nonexistent").unwrap();
        assert!(!not_removed);
    }

    #[test]
    fn test_structured_document_calculate_word_count() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            1,
            "This has four words".to_string(), // 4 words
        )
        .unwrap();

        let child = StructuredSection::new(
            "child".to_string(),
            "Child Section".to_string(),
            2,
            "This has three words".to_string(), // 4 words: This, has, three, words
        )
        .unwrap();

        parent.add_child(child).unwrap();
        doc.add_section(parent).unwrap();

        doc.calculate_total_word_count();
        assert_eq!(doc.word_count, Some(8)); // 4 + 4 = 8
    }

    #[test]
    fn test_structured_document_get_statistics() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let child = StructuredSection::new(
            "child".to_string(),
            "Child Section".to_string(),
            2,
            "Child content".to_string(),
        )
        .unwrap();

        parent.add_child(child).unwrap();
        doc.add_section(parent).unwrap();

        let stats = doc.get_statistics();
        assert_eq!(stats.total_sections, 2); // parent + child
        assert!(stats.total_word_count > 0);
        assert_eq!(stats.max_depth, 2);
        assert_eq!(stats.large_sections_count, 0);
    }

    #[test]
    fn test_structured_document_cleanup() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        // Add section with empty content
        let empty_section = StructuredSection::new(
            "empty".to_string(),
            "Empty Section".to_string(),
            1,
            "   \n\n   ".to_string(), // Only whitespace
        )
        .unwrap();

        // Add section with real content
        let real_section = StructuredSection::new(
            "real".to_string(),
            "Real Section".to_string(),
            1,
            "Real content here".to_string(),
        )
        .unwrap();

        doc.add_section(empty_section).unwrap();
        doc.add_section(real_section).unwrap();
        assert_eq!(doc.total_sections, 2);

        // Cleanup should remove empty section
        let removed_count = doc.cleanup().unwrap();
        assert_eq!(removed_count, 1);
        assert_eq!(doc.total_sections, 1);
        assert!(doc.find_section_by_id("empty").is_none());
        assert!(doc.find_section_by_id("real").is_some());
    }

    #[test]
    fn test_structured_document_memory_usage() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let section = StructuredSection::new(
            "section1".to_string(),
            "Test Section".to_string(),
            1,
            "Test content".to_string(),
        )
        .unwrap();

        doc.add_section(section).unwrap();
        doc.build_index();

        let memory_usage = doc.get_memory_usage();
        assert!(memory_usage.total_content_size > 0);
        assert_eq!(memory_usage.section_count, 1);
        assert!(memory_usage.index_size > 0);
        assert!(memory_usage.estimated_total_size > 0);
    }

    #[test]
    fn test_structured_document_get_all_sections() {
        let mut doc =
            StructuredDocument::new("task1".to_string(), "Test Document".to_string()).unwrap();

        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent Section".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let child1 = StructuredSection::new(
            "child1".to_string(),
            "Child 1".to_string(),
            2,
            "Child 1 content".to_string(),
        )
        .unwrap();

        let child2 = StructuredSection::new(
            "child2".to_string(),
            "Child 2".to_string(),
            2,
            "Child 2 content".to_string(),
        )
        .unwrap();

        parent.add_child(child1).unwrap();
        parent.add_child(child2).unwrap();
        doc.add_section(parent).unwrap();

        let all_sections = doc.get_all_sections();
        assert_eq!(all_sections.len(), 3); // parent + 2 children

        let ids: Vec<&str> = all_sections.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"parent"));
        assert!(ids.contains(&"child1"));
        assert!(ids.contains(&"child2"));
    }

    #[test]
    fn test_structured_section_get_all_children() {
        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let mut child1 = StructuredSection::new(
            "child1".to_string(),
            "Child 1".to_string(),
            2,
            "Child 1 content".to_string(),
        )
        .unwrap();

        let grandchild = StructuredSection::new(
            "grandchild".to_string(),
            "Grandchild".to_string(),
            3,
            "Grandchild content".to_string(),
        )
        .unwrap();

        child1.add_child(grandchild).unwrap();
        parent.add_child(child1).unwrap();

        let all_children = parent.get_all_children();
        assert_eq!(all_children.len(), 2); // child1 + grandchild

        let ids: Vec<&str> = all_children.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"child1"));
        assert!(ids.contains(&"grandchild"));
    }

    #[test]
    fn test_structured_section_depth_calculation() {
        let mut parent = StructuredSection::new(
            "parent".to_string(),
            "Parent".to_string(),
            1,
            "Parent content".to_string(),
        )
        .unwrap();

        let mut child = StructuredSection::new(
            "child".to_string(),
            "Child".to_string(),
            2,
            "Child content".to_string(),
        )
        .unwrap();

        let grandchild = StructuredSection::new(
            "grandchild".to_string(),
            "Grandchild".to_string(),
            3,
            "Grandchild content".to_string(),
        )
        .unwrap();

        // Test depth without children
        assert_eq!(parent.get_depth(), 1);

        // Add child and test depth
        child.add_child(grandchild).unwrap();
        parent.add_child(child).unwrap();
        assert_eq!(parent.get_depth(), 3); // parent -> child -> grandchild
    }
}
