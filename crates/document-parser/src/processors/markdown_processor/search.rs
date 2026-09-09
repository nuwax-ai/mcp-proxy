//! `MarkdownProcessor` 的章节与搜索族方法（从 markdown_processor.rs 拆出）：
//! 章节内容定位、全文搜索、上下文提取与相关度评分。纯代码搬移，无行为变化。

use crate::models::{DocumentStructure, StructuredSection};
use tracing::instrument;

use super::SearchResult;
use std::collections::HashMap;

impl super::MarkdownProcessor {
    /// 获取章节内容
    pub fn get_section_content(
        &self,
        _doc_structure: &DocumentStructure,
        _section_id: &str,
    ) -> Option<String> {
        // 递归查找章节
        // DocumentStructure.sections 是 HashMap<String, String>，不是 StructuredSection
        // 我们需要从其他地方获取章节信息，暂时返回空结果
        None
    }

    /// 递归查找章节
    #[allow(dead_code)]
    fn find_section_recursive<'a>(
        &self,
        sections: &'a HashMap<String, StructuredSection>,
        section_id: &str,
    ) -> Option<&'a StructuredSection> {
        for section in sections.values() {
            if section.id == section_id {
                return Some(section);
            }

            if let Some(found) = self.find_section_recursive(sections, section_id) {
                return Some(found);
            }
        }
        None
    }

    /// 搜索内容
    #[instrument(skip(self, doc_structure, query))]
    pub async fn search_content(
        &self,
        doc_structure: &DocumentStructure,
        query: &str,
    ) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let query_lower = query.to_lowercase();

        // 搜索sections中的内容
        for (section_id, content) in &doc_structure.sections {
            let content_lower = content.to_lowercase();
            if let Some(byte_pos) = content_lower.find(&query_lower) {
                // 找到匹配的TOC项目
                if let Some(toc_item) = doc_structure.toc.iter().find(|item| item.id == *section_id)
                {
                    // 将字节位置转换为字符位置
                    let char_pos = content[..byte_pos].chars().count();
                    let relevance_score = self.calculate_relevance_score(query, content, char_pos);
                    let context = self.extract_context(content, char_pos, query.chars().count());

                    results.push(SearchResult {
                        section_id: section_id.clone(),
                        title: toc_item.title.clone(),
                        content: content.clone(),
                        context,
                        relevance_score,
                        position: char_pos,
                    });
                }
            }
        }

        // 按相关性排序
        results.sort_by(|a: &SearchResult, b: &SearchResult| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        results
    }

    /// 递归搜索章节
    #[allow(dead_code)]
    fn search_sections_recursive(
        &self,
        sections: &[StructuredSection],
        query: &str,
        results: &mut Vec<SearchResult>,
    ) {
        for section in sections {
            let content_lower = section.content.to_lowercase();

            if content_lower.contains(query) {
                let position = content_lower.find(query).unwrap_or(0);
                let relevance_score =
                    self.calculate_relevance_score(query, &section.content, position);

                results.push(SearchResult {
                    section_id: section.id.clone(),
                    title: section.title.clone(),
                    content: section.content.clone(),
                    context: self.extract_context(&section.content, position, query.len()),
                    position,
                    relevance_score,
                });
            }

            // 递归搜索子章节
            let children_slice: Vec<StructuredSection> = section
                .children
                .iter()
                .map(|boxed| boxed.as_ref().clone())
                .collect();
            self.search_sections_recursive(&children_slice, query, results);
        }
    }

    /// 提取上下文
    fn extract_context(&self, content: &str, position: usize, query_len: usize) -> String {
        let chars: Vec<char> = content.chars().collect();
        let start = position.saturating_sub(50);
        let end = (position + query_len + 50).min(chars.len());

        let context: String = chars[start..end].iter().collect();

        if start > 0 {
            format!("...{context}...")
        } else {
            format!("{context}...")
        }
    }

    /// 计算相关性分数
    fn calculate_relevance_score(&self, query: &str, content: &str, position: usize) -> f64 {
        let mut score = 0.0;

        // 位置分数（越靠前分数越高）
        let position_score = 1.0 - (position as f64 / content.len() as f64);
        score += position_score * 0.3;

        // 匹配次数分数
        let matches = content.to_lowercase().matches(query).count();
        let frequency_score = (matches as f64).min(10.0) / 10.0;
        score += frequency_score * 0.4;

        // 内容长度分数（适中的长度分数更高）
        let length_score = if content.len() > 100 && content.len() < 1000 {
            1.0
        } else {
            0.5
        };
        score += length_score * 0.3;

        score
    }
}
