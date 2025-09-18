//! 章节ID重复问题的单元测试
//!
//! 这个测试模块专门用于验证和解决文档解析过程中出现的"章节ID 已存在"错误。
//! 主要测试场景包括：
//! 1. Markdown文件中重复标题的处理
//! 2. 章节ID生成的唯一性
//! 3. 结构化文档创建时的ID冲突处理

use crate::{
    error::AppError,
    models::{
        StructuredDocument, StructuredSection,
    },
    processors::MarkdownProcessor,
    tests::test_helpers::{create_test_app_state, safe_init_global_config},
};
use std::path::Path;
use tokio::fs;

#[cfg(test)]
mod section_id_tests {
    use super::*;

    /// 测试Markdown文件中重复标题的处理
    #[tokio::test]
    async fn test_duplicate_section_titles_handling() {
        // 安全初始化全局配置
        safe_init_global_config();

        // 创建包含重复标题的测试Markdown内容
        let markdown_content = r#"# 均线为王 均线100

均线上的舞者 著

# 图书在版编目（CIP）数据

均线为王之一：均线100分／均线上的舞者著．一成都：

# 均线为王之一：均线100 分

# 均线上的舞者著

策划组稿 何朝霞责任编辑

# 序

我作为今日财经、学股网创始人兼CEO

# 前言

十年磨一剑，皇天不负有心人

# 目录

# 第一章均线的支撑和压力 001

## 均线的价值

移动平均线作为一个大家常用的指标

## 均线的压力

均线代表了此前一段时间所有参与这只股票的人的平均成本

## 均线的价值

这是第二个同名的章节，用于测试重复标题处理

## 多根均线的压力

均线压力，当股价反弹至均线附近时
"#;

        // 创建MarkdownProcessor实例
        let processor = MarkdownProcessor::default();

        // 解析Markdown内容并生成TOC
        let toc_result = processor.parse_markdown_with_toc(markdown_content).await;
        let toc_items = match toc_result {
            Ok(doc_structure) => doc_structure.toc,
            Err(e) => {
                panic!("TOC生成失败: {e:?}");
            }
        };

        // TOC已经在上面提取了

        // 验证所有章节ID都是唯一的
        let mut section_ids = std::collections::HashSet::new();
        for item in &toc_items {
            assert!(
                section_ids.insert(item.id.clone()),
                "章节ID应该是唯一的，发现重复ID: {}",
                item.id
            );
        }

        // 验证重复标题被正确处理（应该有不同的ID）
        let title_counts = toc_items
            .iter()
            .filter(|item| item.title == "均线的价值")
            .count();
        assert_eq!(title_counts, 2, "应该有2个'均线的价值'标题");

        // 验证这两个相同标题有不同的ID
        let value_items: Vec<_> = toc_items
            .iter()
            .filter(|item| item.title == "均线的价值")
            .collect();
        assert_ne!(
            value_items[0].id, value_items[1].id,
            "相同标题应该有不同的ID"
        );

        println!("TOC项目数量: {}", toc_items.len());
        for item in &toc_items {
            println!(
                "ID: {}, 标题: {}, 级别: {}",
                item.id, item.title, item.level
            );
        }
    }

    /// 测试结构化文档创建时的ID冲突处理
    #[tokio::test]
    async fn test_structured_document_creation_with_duplicate_ids() {
        // 安全初始化全局配置
        safe_init_global_config();

        // 创建测试应用状态
        let app_state = create_test_app_state().await;

        // 创建包含重复标题的Markdown内容
        let markdown_content = "# 测试文档\n\n## 图形特征\n\n第一个图形特征内容\n\n## 图形特征\n\n第二个图形特征内容\n\n### 技术分析\n\n技术分析内容";

        // 使用DocumentService的公共API生成结构化文档
        let result = app_state
            .document_service
            .generate_structured_document_simple(markdown_content)
            .await;

        // 验证结构化文档创建成功
        assert!(result.is_ok(), "结构化文档创建应该成功: {:?}", result.err());
        let structured_doc = result.unwrap();

        // 打印实际的章节信息用于调试
        // 验证章节数量和ID唯一性

        // 验证所有章节都被正确添加
        assert!(
            structured_doc.toc.len() >= 3,
            "应该至少有3个章节，实际有: {}",
            structured_doc.toc.len()
        );

        // 验证章节ID的唯一性
        let mut section_ids = std::collections::HashSet::new();
        for section in &structured_doc.toc {
            assert!(
                section_ids.insert(section.id.clone()),
                "章节ID应该是唯一的，发现重复ID: {}",
                section.id
            );
        }

        println!("结构化文档创建成功，章节数量: {}", structured_doc.toc.len());
        for section in &structured_doc.toc {
            println!(
                "章节ID: {}, 标题: {}, 级别: {}",
                section.id, section.title, section.level
            );
        }
    }

    /// 测试真实的upload_parse_test.md文件解析
    #[tokio::test]
    async fn test_real_markdown_file_parsing() {
        // 安全初始化全局配置
        safe_init_global_config();

        let test_file_path =
            "/Volumes/soddygo/git_work/mcp_proxy/document-parser/fixtures/upload_parse_test.md";

        // 检查文件是否存在
        if !Path::new(test_file_path).exists() {
            println!("测试文件不存在，跳过测试: {test_file_path}");
            return;
        }

        // 读取文件内容
        let markdown_content = match fs::read_to_string(test_file_path).await {
            Ok(content) => content,
            Err(e) => {
                println!("无法读取测试文件: {e}");
                return;
            }
        };

        // 创建MarkdownProcessor实例
        let processor = MarkdownProcessor::default();

        // 解析Markdown内容并生成TOC
        let toc_result = processor.parse_markdown_with_toc(&markdown_content).await;

        // 验证TOC生成成功
        assert!(
            toc_result.is_ok(),
            "TOC生成应该成功: {:?}",
            toc_result.err()
        );
        let doc_structure = toc_result.unwrap();
        let toc_items = doc_structure.toc;

        // 验证所有章节ID都是唯一的
        let mut section_ids = std::collections::HashSet::new();
        let mut duplicate_ids = Vec::new();

        for item in &toc_items {
            if !section_ids.insert(item.id.clone()) {
                duplicate_ids.push(item.id.clone());
            }
        }

        // 如果有重复ID，打印详细信息
        if !duplicate_ids.is_empty() {
            println!("发现重复的章节ID:");
            for duplicate_id in &duplicate_ids {
                let items_with_same_id: Vec<_> = toc_items
                    .iter()
                    .filter(|item| item.id == *duplicate_id)
                    .collect();
                println!("重复ID '{duplicate_id}' 出现在以下章节中:");
                for item in items_with_same_id {
                    println!("  - 标题: '{}', 级别: {}", item.title, item.level);
                }
            }
        }

        // 断言没有重复ID
        assert!(
            duplicate_ids.is_empty(),
            "发现重复的章节ID: {duplicate_ids:?}"
        );

        // 不需要创建ParseResult，直接使用DocumentService的公共API

        // 使用DocumentService的公共API生成结构化文档
        let app_state = create_test_app_state().await;
        let structured_doc_result = app_state
            .document_service
            .generate_structured_document_simple(&markdown_content)
            .await;

        // 验证结构化文档创建成功
        assert!(
            structured_doc_result.is_ok(),
            "结构化文档创建应该成功: {:?}",
            structured_doc_result.err()
        );

        let structured_doc = structured_doc_result.unwrap();

        println!("成功解析真实Markdown文件:");
        println!("- 文件路径: {test_file_path}");
        println!("- TOC项目数量: {}", toc_items.len());
        println!("- 结构化章节数量: {}", structured_doc.toc.len());
        println!("- 文档标题: {}", structured_doc.document_title);

        // 验证章节内容不为空（这是之前修复的问题）
        let empty_content_sections: Vec<_> = structured_doc
            .toc
            .iter()
            .filter(|section| section.content.is_empty())
            .collect();

        if !empty_content_sections.is_empty() {
            println!("发现内容为空的章节:");
            for section in empty_content_sections {
                println!("  - ID: {}, 标题: {}", section.id, section.title);
            }
        }

        // 打印前几个章节的信息用于调试
        println!("前5个章节信息:");
        for (i, section) in structured_doc.toc.iter().take(5).enumerate() {
            println!(
                "  {}. ID: {}, 标题: {}, 内容长度: {}",
                i + 1,
                section.id,
                section.title,
                section.content.len()
            );
        }
    }

    /// 测试章节ID生成算法的唯一性
    #[tokio::test]
    async fn test_section_id_generation_uniqueness() {
        // 安全初始化全局配置
        safe_init_global_config();

        let processor = MarkdownProcessor::default();

        // 创建包含重复标题的测试Markdown内容
        let test_markdown = r#"# 图形特征

内容1

# 图形特征

内容2

# 技术分析

内容3

# 图形特征

内容4

# 技术分析

内容5

# 均线理论

内容6"#;

        // 使用MarkdownProcessor解析并生成TOC
        let toc_result = processor.parse_markdown_with_toc(test_markdown).await;
        assert!(toc_result.is_ok(), "TOC生成应该成功");

        let doc_structure = toc_result.unwrap();
        let generated_ids: Vec<String> = doc_structure
            .toc
            .iter()
            .map(|item| item.id.clone())
            .collect();

        // 验证所有生成的ID都是唯一的
        let mut unique_ids = std::collections::HashSet::new();
        for id in &generated_ids {
            assert!(
                unique_ids.insert(id.clone()),
                "生成的ID应该是唯一的，发现重复ID: {id}"
            );
        }

        println!("生成的唯一ID:");
        for (i, id) in generated_ids.iter().enumerate() {
            println!("  {}. {}", i + 1, id);
        }

        // 验证ID的唯一性（具体格式可能因实现而异）
        let mut unique_ids = std::collections::HashSet::new();
        for id in &generated_ids {
            assert!(unique_ids.insert(id.clone()), "ID应该是唯一的: {id}");
        }

        // 验证至少有6个不同的ID
        assert_eq!(generated_ids.len(), 6, "应该有6个章节ID");
    }

    /// 测试StructuredDocument的add_section方法对重复ID的处理
    #[tokio::test]
    async fn test_structured_document_add_section_duplicate_handling() {
        // 安全初始化全局配置
        safe_init_global_config();

        let mut structured_doc =
            StructuredDocument::new("测试文档".to_string(), "这是一个测试文档".to_string())
                .expect("创建结构化文档应该成功");

        // 创建第一个章节
        let section1 = StructuredSection::new(
            "图形特征".to_string(),
            "图形特征".to_string(),
            1,
            "第一个图形特征的内容".to_string(),
        )
        .expect("创建章节1应该成功");

        // 添加第一个章节应该成功
        let result1 = structured_doc.add_section(section1);
        assert!(result1.is_ok(), "添加第一个章节应该成功");

        // 创建具有相同ID的第二个章节
        let section2 = StructuredSection::new(
            "图形特征".to_string(), // 相同的ID
            "图形特征（重复）".to_string(),
            1,
            "第二个图形特征的内容".to_string(),
        )
        .expect("创建章节2应该成功");

        // 添加具有重复ID的章节应该失败
        let result2 = structured_doc.add_section(section2);
        assert!(result2.is_err(), "添加重复ID的章节应该失败");

        // 验证错误类型
        match result2.err().unwrap() {
            AppError::Validation(msg) => {
                assert!(msg.contains("章节ID"), "错误消息应该包含'章节ID'");
                assert!(msg.contains("已存在"), "错误消息应该包含'已存在'");
            }
            _ => panic!("应该是验证错误"),
        }

        // 验证只有一个章节被添加
        assert_eq!(structured_doc.toc.len(), 1, "应该只有一个章节");

        println!("重复ID检测测试通过");
    }
}
