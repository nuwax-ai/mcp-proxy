use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 目录项数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TocItem {
    pub id: String,
    pub title: String,
    pub level: u8,
    pub anchor: String,
    pub start_pos: usize,
    pub end_pos: usize,
    pub children: Vec<TocItem>,
    pub parent_id: Option<String>,
    pub content_preview: Option<String>,
    pub word_count: Option<usize>,
}

/// 文档结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentStructure {
    pub title: String,
    pub toc: Vec<TocItem>,
    pub sections: HashMap<String, String>, // section_id -> content
    pub total_sections: usize,
    pub max_level: u8,
}

impl TocItem {
    /// 创建新的目录项
    pub fn new(
        id: String,
        title: String,
        level: u8,
        start_pos: usize,
        end_pos: usize,
    ) -> Self {
        let anchor = Self::generate_anchor_id(&title);
        
        Self {
            id,
            title,
            level,
            anchor,
            start_pos,
            end_pos,
            children: Vec::new(),
            parent_id: None,
            content_preview: None,
            word_count: None,
        }
    }

    /// 生成锚点ID
    pub fn generate_anchor_id(title: &str) -> String {
        title
            .to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() {
                    c
                } else if c.is_whitespace() || c == '-' || c == '_' {
                    '-'
                } else {
                    '_'
                }
            })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("-")
    }

    /// 添加子项
    pub fn add_child(&mut self, mut child: TocItem) {
        child.parent_id = Some(self.id.clone());
        self.children.push(child);
    }

    /// 设置内容预览
    pub fn set_content_preview(&mut self, content: &str, max_length: usize) {
        let preview = if content.len() > max_length {
            format!("{}...", &content[..max_length])
        } else {
            content.to_string()
        };
        self.content_preview = Some(preview);
        self.word_count = Some(content.split_whitespace().count());
    }

    /// 获取所有子项（递归）
    pub fn get_all_children(&self) -> Vec<&TocItem> {
        let mut result = Vec::new();
        for child in &self.children {
            result.push(child);
            result.extend(child.get_all_children());
        }
        result
    }

    /// 获取深度
    pub fn get_depth(&self) -> usize {
        if self.children.is_empty() {
            0
        } else {
            1 + self.children.iter().map(|c| c.get_depth()).max().unwrap_or(0)
        }
    }

    /// 是否有子项
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    /// 获取路径（从根到当前节点）
    pub fn get_path(&self) -> String {
        if let Some(parent_id) = &self.parent_id {
            format!("{} > {}", parent_id, self.title)
        } else {
            self.title.clone()
        }
    }

    /// 查找子项
    pub fn find_child_by_id(&self, id: &str) -> Option<&TocItem> {
        for child in &self.children {
            if child.id == id {
                return Some(child);
            }
            if let Some(found) = child.find_child_by_id(id) {
                return Some(found);
            }
        }
        None
    }

    /// 获取内容范围
    pub fn get_content_range(&self) -> (usize, usize) {
        (self.start_pos, self.end_pos)
    }

    /// 验证位置有效性
    pub fn is_valid_position(&self) -> bool {
        self.start_pos <= self.end_pos
    }
}

impl DocumentStructure {
    /// 创建新的文档结构
    pub fn new(title: String) -> Self {
        Self {
            title,
            toc: Vec::new(),
            sections: HashMap::new(),
            total_sections: 0,
            max_level: 0,
        }
    }

    /// 添加目录项
    pub fn add_toc_item(&mut self, item: TocItem) {
        self.max_level = self.max_level.max(item.level);
        self.total_sections += 1;
        self.toc.push(item);
    }

    /// 添加章节内容
    pub fn add_section(&mut self, section_id: String, content: String) {
        self.sections.insert(section_id, content);
    }

    /// 获取章节内容
    pub fn get_section(&self, section_id: &str) -> Option<&String> {
        self.sections.get(section_id)
    }

    /// 查找目录项
    pub fn find_toc_item(&self, id: &str) -> Option<&TocItem> {
        for item in &self.toc {
            if item.id == id {
                return Some(item);
            }
            if let Some(found) = item.find_child_by_id(id) {
                return Some(found);
            }
        }
        None
    }

    /// 获取指定层级的目录项
    pub fn get_items_by_level(&self, level: u8) -> Vec<&TocItem> {
        let mut result = Vec::new();
        self.collect_items_by_level(&self.toc, level, &mut result);
        result
    }

    fn collect_items_by_level<'a>(
        &'a self,
        items: &'a [TocItem],
        target_level: u8,
        result: &mut Vec<&'a TocItem>,
    ) {
        for item in items {
            if item.level == target_level {
                result.push(item);
            }
            self.collect_items_by_level(&item.children, target_level, result);
        }
    }

    /// 获取统计信息
    pub fn get_statistics(&self) -> DocumentStatistics {
        let total_words = self.sections.values()
            .map(|content| content.split_whitespace().count())
            .sum();
        
        DocumentStatistics {
            total_sections: self.total_sections,
            max_level: self.max_level,
            total_words,
            sections_by_level: self.get_sections_count_by_level(),
        }
    }

    fn get_sections_count_by_level(&self) -> HashMap<u8, usize> {
        let mut counts = HashMap::new();
        for level in 1..=self.max_level {
            let count = self.get_items_by_level(level).len();
            counts.insert(level, count);
        }
        counts
    }

    /// 验证结构完整性
    pub fn validate(&self) -> Result<(), String> {
        // 检查目录项位置有效性
        for item in &self.toc {
            if !item.is_valid_position() {
                return Err(format!("Invalid position for item: {}", item.id));
            }
            if let Err(e) = self.validate_item_recursive(item) {
                return Err(e);
            }
        }
        
        // 检查章节内容完整性
        for item in &self.toc {
            if !self.sections.contains_key(&item.id) {
                return Err(format!("Missing content for section: {}", item.id));
            }
        }
        
        Ok(())
    }

    fn validate_item_recursive(&self, item: &TocItem) -> Result<(), String> {
        for child in &item.children {
            if !child.is_valid_position() {
                return Err(format!("Invalid position for child item: {}", child.id));
            }
            if child.level <= item.level {
                return Err(format!("Invalid level hierarchy for item: {}", child.id));
            }
            if let Err(e) = self.validate_item_recursive(child) {
                return Err(e);
            }
        }
        Ok(())
    }
}

/// 文档统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentStatistics {
    pub total_sections: usize,
    pub max_level: u8,
    pub total_words: usize,
    pub sections_by_level: HashMap<u8, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anchor_generation() {
        assert_eq!(TocItem::generate_anchor_id("第一章 介绍"), "第一章-介绍");
        assert_eq!(TocItem::generate_anchor_id("1.1 Background"), "1_1-background");
        assert_eq!(TocItem::generate_anchor_id("API设计 & 实现"), "api设计-_-实现");
    }

    #[test]
    fn test_toc_item_creation() {
        let item = TocItem::new(
            "section-1".to_string(),
            "第一章".to_string(),
            1,
            0,
            100,
        );
        
        assert_eq!(item.id, "section-1");
        assert_eq!(item.title, "第一章");
        assert_eq!(item.level, 1);
        assert_eq!(item.anchor, "第一章");
        assert!(item.is_valid_position());
    }

    #[test]
    fn test_document_structure() {
        let mut doc = DocumentStructure::new("测试文档".to_string());
        
        let item1 = TocItem::new("s1".to_string(), "章节1".to_string(), 1, 0, 50);
        let item2 = TocItem::new("s2".to_string(), "章节2".to_string(), 1, 51, 100);
        
        doc.add_toc_item(item1);
        doc.add_toc_item(item2);
        doc.add_section("s1".to_string(), "章节1的内容".to_string());
        doc.add_section("s2".to_string(), "章节2的内容".to_string());
        
        assert_eq!(doc.total_sections, 2);
        assert_eq!(doc.max_level, 1);
        assert!(doc.validate().is_ok());
    }
}