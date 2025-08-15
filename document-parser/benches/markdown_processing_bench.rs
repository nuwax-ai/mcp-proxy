use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use document_parser::models::StructuredDocument;
use document_parser::processors::{MarkdownProcessor, MarkdownProcessorConfig};
use tokio::runtime::Runtime;

fn markdown_processing_benchmark(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("markdown_processing");
    
    // 测试不同大小的文档
    let small_doc = generate_test_document(100);   // ~100 lines
    let medium_doc = generate_test_document(1000); // ~1000 lines  
    let large_doc = generate_test_document(10000); // ~10000 lines
    
    let processor = MarkdownProcessor::new(MarkdownProcessorConfig::default());
    let streaming_processor = MarkdownProcessor::new(MarkdownProcessorConfig {
        large_document_threshold: 1000, // 1KB threshold for testing
        ..Default::default()
    });
    
    // 基准测试：创建结构化文档
    group.bench_function("create_structured_document", |b| {
        b.iter(|| {
            let mut doc = StructuredDocument::new(
                black_box("test-task-id".to_string()),
                black_box("测试文档".to_string()),
            ).unwrap();
            
            // 添加一些测试章节
            for i in 1..=10 {
                let section = document_parser::models::StructuredSection::new(
                    format!("section-{}", i),
                    format!("第{}章 测试", i),
                    (i % 3 + 1) as u8,
                    format!("这是第{}章的内容，包含一些测试文本。", i),
                ).unwrap();
                doc.add_section(section).unwrap();
            }
            
            doc.calculate_total_word_count();
            doc
        });
    });
    
    // 基准测试：标准Markdown解析
    for (name, content) in [
        ("small", &small_doc),
        ("medium", &medium_doc),
        ("large", &large_doc),
    ] {
        group.bench_with_input(
            BenchmarkId::new("parse_markdown_standard", name),
            content,
            |b, content| {
                b.to_async(&rt).iter(|| async {
                    processor.parse_markdown_with_toc(black_box(content)).await.unwrap()
                });
            },
        );
    }
    
    // 基准测试：流式Markdown解析
    for (name, content) in [
        ("medium", &medium_doc),
        ("large", &large_doc),
    ] {
        group.bench_with_input(
            BenchmarkId::new("parse_markdown_streaming", name),
            content,
            |b, content| {
                b.to_async(&rt).iter(|| async {
                    streaming_processor.parse_markdown_with_toc(black_box(content)).await.unwrap()
                });
            },
        );
    }
    
    // 基准测试：TOC提取
    group.bench_with_input(
        BenchmarkId::new("extract_toc", "medium"),
        &medium_doc,
        |b, content| {
            b.to_async(&rt).iter(|| async {
                processor.extract_table_of_contents(black_box(content)).await.unwrap()
            });
        },
    );
    
    // 基准测试：内容搜索
    let doc_structure = rt.block_on(processor.parse_markdown_with_toc(&medium_doc)).unwrap();
    group.bench_function("search_content", |b| {
        b.to_async(&rt).iter(|| async {
            processor.search_content(black_box(&doc_structure), black_box("测试")).await
        });
    });
    
    // 基准测试：缓存性能
    group.bench_with_input(
        BenchmarkId::new("parse_with_cache", "medium"),
        &medium_doc,
        |b, content| {
            b.to_async(&rt).iter(|| async {
                // 第一次解析会缓存，第二次应该从缓存读取
                processor.parse_markdown_with_toc(black_box(content)).await.unwrap();
                processor.parse_markdown_with_toc(black_box(content)).await.unwrap()
            });
        },
    );
    
    // 基准测试：批量处理
    let batch_docs: Vec<(String, String)> = (0..10)
        .map(|i| (format!("doc-{}", i), generate_test_document(100)))
        .collect();
    
    group.bench_function("batch_process", |b| {
        b.to_async(&rt).iter(|| async {
            processor.batch_process_documents(black_box(batch_docs.clone())).await.unwrap()
        });
    });
    
    group.finish();
}

/// 生成测试文档
fn generate_test_document(lines: usize) -> String {
    let mut content = String::with_capacity(lines * 50);
    content.push_str("# 测试文档\n\n这是一个测试文档的介绍。\n\n");
    
    for i in 1..=(lines / 10) {
        content.push_str(&format!("## 第{}章\n\n", i));
        content.push_str(&format!("这是第{}章的内容。", i));
        content.push_str("包含一些测试文本和示例代码。\n\n");
        
        for j in 1..=3 {
            content.push_str(&format!("### {}.{} 小节\n\n", i, j));
            content.push_str("这是小节的内容，包含：\n\n");
            content.push_str("- 列表项1\n");
            content.push_str("- 列表项2\n");
            content.push_str("- 列表项3\n\n");
            content.push_str("```rust\n");
            content.push_str("fn example() {\n");
            content.push_str("    println!(\"Hello, world!\");\n");
            content.push_str("}\n");
            content.push_str("```\n\n");
        }
    }
    
    content
}

criterion_group!(benches, markdown_processing_benchmark);
criterion_main!(benches);
