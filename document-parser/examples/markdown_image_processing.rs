use document_parser::config::init_global_config;
use document_parser::models::ImageInfo;
use document_parser::parsers::DualEngineParser;
use document_parser::processors::MarkdownProcessor;
use document_parser::processors::markdown_processor::MarkdownProcessorConfig;
use document_parser::services::{DocumentService, ImageProcessor, TaskService};
use std::sync::Arc;
use tokio::fs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Markdown 图片处理核心逻辑测试");
    println!("=====================================");

    // 初始化全局配置
    let config =
        document_parser::config::AppConfig::load_config().expect("Failed to load configuration");
    init_global_config(config).expect("Failed to initialize global config");
    println!("✅ 全局配置初始化完成");

    // 获取项目根目录
    let project_root = std::env::current_dir()?;
    let test_file_path = project_root
        .join("document-parser")
        .join("fixtures")
        .join("upload_parse_test.md");

    println!("📁 测试文件路径: {}", test_file_path.display());

    // 检查测试文件是否存在
    if !test_file_path.exists() {
        eprintln!("❌ 测试文件不存在: {}", test_file_path.display());
        return Ok(());
    }

    // 读取测试 Markdown 文件
    let markdown_content = fs::read_to_string(&test_file_path).await?;
    println!(
        "📖 读取 Markdown 文件成功，内容长度: {} 字符",
        markdown_content.len()
    );

    // 创建 Markdown 处理器
    let processor = MarkdownProcessor::new(MarkdownProcessorConfig::default(), None);
    println!("🔧 Markdown 处理器创建完成");

    // 测试 1: 解析 Markdown 并构建章节层次结构
    println!("\n🧪 测试 1: 解析 Markdown 并构建章节层次结构");
    let doc_structure = processor.parse_markdown_with_toc(&markdown_content).await?;

    println!("   文档标题: {}", doc_structure.title);
    println!("   总章节数: {}", doc_structure.total_sections);
    println!("   最大层级: {}", doc_structure.max_level);
    println!("   TOC 项目数: {}", doc_structure.toc.len());

    // 显示前几个 TOC 项目
    for (i, item) in doc_structure.toc.iter().take(5).enumerate() {
        println!(
            "   {}. [{}] {} (层级: {})",
            i + 1,
            item.id,
            item.title,
            item.level
        );
    }

    // 测试 2: 提取图片路径
    println!("\n🧪 测试 2: 提取 Markdown 中的图片路径");
    let image_paths = ImageProcessor::extract_image_paths(&markdown_content);
    println!("   发现图片路径数量: {}", image_paths.len());

    // 只显示前10个图片路径
    for (i, path) in image_paths.iter().take(10).enumerate() {
        println!("   {}. {}", i + 1, path);
    }
    if image_paths.len() > 10 {
        println!("   ... 还有 {} 个图片路径", image_paths.len() - 10);
    }

    // 测试 3: 验证图片文件是否存在（修复路径匹配问题）
    println!("\n🧪 测试 3: 验证图片文件是否存在");

    // 根据测试文件路径确定图片目录位置
    let images_dir = if test_file_path.parent().unwrap().join("images").exists() {
        test_file_path.parent().unwrap().join("images")
    } else {
        // 回退到默认位置
        project_root
            .join("document-parser")
            .join("fixtures")
            .join("images")
    };
    let mut existing_images = 0;
    let mut missing_images = 0;
    let mut valid_image_paths = Vec::new();

    for image_name in &image_paths {
        // 现在 image_paths 直接包含图片名称（如 filename.jpg）
        let filename = image_name;

        // 检查文件是否存在
        let full_path = images_dir.join(filename);
        if full_path.exists() {
            let metadata = fs::metadata(&full_path).await?;
            println!("   ✅ {} ({} bytes)", filename, metadata.len());
            existing_images += 1;
            valid_image_paths.push(image_name.clone());
        } else {
            // 如果直接匹配失败，尝试在 images 目录中查找
            let mut found = false;
            if let Ok(mut entries) = fs::read_dir(&images_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let entry_path = entry.path();
                    if let Some(entry_filename) = entry_path.file_name().and_then(|f| f.to_str()) {
                        if entry_filename == filename {
                            let metadata = fs::metadata(&entry_path).await?;
                            println!(
                                "   ✅ {} ({} bytes) [通过文件名匹配]",
                                filename,
                                metadata.len()
                            );
                            existing_images += 1;
                            valid_image_paths.push(image_name.clone());
                            found = true;
                            break;
                        }
                    }
                }
            }

            if !found {
                println!("   ❌ {filename} (文件不存在)");
                missing_images += 1;
            }
        }
    }

    println!("   存在图片: {existing_images} 个");
    println!("   缺失图片: {missing_images} 个");

    // 测试 4: 创建真实的图片上传结果（基于实际存在的图片）
    println!("\n🧪 测试 4: 创建真实的图片上传结果");

    let mut real_image_results = Vec::new();
    for image_name in &valid_image_paths {
        // 现在 valid_image_paths 直接包含图片名称（如 filename.jpg）
        let filename = image_name;

        // 模拟真实的 OSS URL（实际项目中这里会是真实的 OSS 上传结果）
        let oss_url = format!(
            "https://example-oss.com/processed_images/{}/{}",
            uuid::Uuid::new_v4().to_string().split('-').next().unwrap(),
            filename
        );

        // 获取实际文件大小
        let full_path = images_dir.join(filename);
        let file_size = fs::metadata(&full_path).await?.len() as u64;

        // 为了创建 ImageInfo，我们需要构建完整的原始路径
        let original_path = format!("images/{image_name}");

        real_image_results.push(ImageInfo::new(
            original_path,
            oss_url,
            file_size,
            "image/jpeg".to_string(),
        ));
    }

    println!("   创建真实图片结果: {} 个", real_image_results.len());

    // 测试 5: 测试 Markdown 内容替换
    if !real_image_results.is_empty() {
        println!("\n🧪 测试 5: 测试 Markdown 内容替换");

        // 创建临时的 DocumentService 来测试替换逻辑
        let temp_oss_service = None; // 不使用真实的 OSS 服务
        let temp_task_service = Arc::new(
            TaskService::new(Arc::new(
                sled::open(":memory:").expect("Failed to create in-memory DB"),
            ))
            .expect("Failed to create task service"),
        );
        let temp_dual_parser = DualEngineParser::with_auto_venv_detection()
            .expect("Failed to create dual engine parser");
        let temp_markdown_processor =
            MarkdownProcessor::new(MarkdownProcessorConfig::default(), None);

        let temp_doc_service = DocumentService::new(
            temp_dual_parser,
            temp_markdown_processor,
            temp_task_service,
            temp_oss_service,
        );

        // 测试路径替换逻辑
        let replaced_content = temp_doc_service
            .replace_image_paths_in_markdown(&markdown_content, &real_image_results)
            .await?;

        println!("   原始内容长度: {} 字符", markdown_content.len());
        println!("   替换后内容长度: {} 字符", replaced_content.len());

        // 检查是否成功替换了图片路径
        let original_image_count = image_paths.len();
        let replaced_image_count = ImageProcessor::extract_image_paths(&replaced_content).len();

        if replaced_image_count == 0 {
            println!("   ✅ 图片路径替换成功，所有本地路径已替换为 OSS URL");
        } else {
            println!("   ⚠️  仍有 {replaced_image_count} 个图片路径未替换");
        }

        // 显示替换前后的对比（前几行）
        println!("\n📝 内容替换对比 (前 10 行):");
        println!("   原始内容:");
        for (i, line) in markdown_content.lines().take(10).enumerate() {
            if line.contains("![") || line.contains("](") {
                println!("   {}: {}", i + 1, line);
            }
        }

        println!("   替换后内容:");
        for (i, line) in replaced_content.lines().take(10).enumerate() {
            if line.contains("![") || line.contains("](") {
                println!("   {}: {}", i + 1, line);
            }
        }

        // 测试 6: 验证替换结果
        println!("\n🧪 测试 6: 验证替换结果");
        let replaced_image_paths = ImageProcessor::extract_image_paths(&replaced_content);
        let oss_url_count = replaced_content.matches("https://example-oss.com").count();

        println!("   替换后图片路径数量: {}", replaced_image_paths.len());
        println!("   OSS URL 数量: {oss_url_count}");

        if oss_url_count > 0 {
            println!("   ✅ 成功替换了 {oss_url_count} 个图片路径为 OSS URL");
        } else {
            println!("   ❌ 没有找到 OSS URL，替换可能失败");
        }
    }

    // 测试总结
    println!("\n📊 测试总结");
    println!("=====================================");
    println!("✅ Markdown 解析: 成功");
    println!("✅ 章节层次结构: {} 个章节", doc_structure.total_sections);
    println!("✅ 图片路径提取: {} 个图片", image_paths.len());
    println!(
        "✅ 图片文件验证: {}/{} 个文件存在",
        existing_images,
        image_paths.len()
    );
    println!("✅ 路径替换测试: 完成");

    if missing_images > 0 {
        println!("⚠️  缺失图片: {missing_images} 个 (需要检查图片文件)");
    }

    if !real_image_results.is_empty() {
        println!("✅ 图片上传模拟: {} 个图片", real_image_results.len());
        println!("✅ Markdown 内容替换: 完成");
    }

    println!("\n🎉 核心逻辑测试完成！");
    println!("\n💡 注意：这是一个测试程序，实际 OSS 上传需要：");
    println!("   1. 配置真实的 OSS 服务");
    println!("   2. 调用 ImageProcessor::batch_upload_images 方法");
    println!("   3. 使用真实的 OSS 凭证");

    Ok(())
}
