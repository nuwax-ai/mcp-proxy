use anyhow::Result;
use super::{DownloadArgs, ListArgs};
use crate::models::{list_available_models, ModelInfo};

/// 执行模型下载
pub async fn download_model(args: DownloadArgs) -> Result<()> {
    use fastembed::{TextEmbedding, InitOptions};
    
    tracing::info!("开始下载模型...");
    
    // 解析模型
    let model = if let Some(model_name) = args.model {
        // 使用内置模型变体名
        crate::models::parse_model(&model_name)?
    } else if let Some(code) = args.code {
        // 使用模型代码
        crate::models::parse_model(&code)?
    } else {
        anyhow::bail!("必须指定 --model 或 --code 参数");
    };
    
    // 显示下载信息
    let model_info = ModelInfo::from_embedding_model(&model);
    println!("📦 下载模型:");
    println!("   变体名: {}", model_info.variant);
    println!("   模型代码: {}", model_info.code);
    println!("   向量维度: {}", model_info.dim);
    println!("   缓存目录: {}", args.cache_dir.display());
    println!();
    
    // 初始化模型（会自动下载）
    let mut options = InitOptions::new(model.clone());
    options = options.with_cache_dir(args.cache_dir.clone());
    options = options.with_show_download_progress(args.progress);
    
    println!("⬇️  正在下载模型文件...");
    let start = std::time::Instant::now();
    
    let _embedding = TextEmbedding::try_new(options)?;
    
    let elapsed = start.elapsed();
    
    println!();
    println!("✅ 模型下载完成!");
    println!("   耗时: {:?}", elapsed);
    println!("   缓存位置: {}", args.cache_dir.display());
    
    // 验证文件
    println!();
    println!("🔍 验证模型文件...");
    let available = list_available_models(args.cache_dir.to_str().unwrap())?;
    
    if available.iter().any(|m| m.variant == model_info.variant) {
        println!("✅ 模型文件验证成功!");
    } else {
        println!("⚠️  警告: 模型文件可能不完整");
    }
    
    Ok(())
}

/// 列出已下载的模型
pub async fn list_models(args: ListArgs) -> Result<()> {
    use crate::models::list_available_models;
    
    println!("📋 查询已下载的模型...");
    println!("   类型: {}", args.r#type);
    println!("   缓存目录: {}", args.cache_dir.display());
    println!();
    
    // 检查缓存目录是否存在
    if !args.cache_dir.exists() {
        println!("⚠️  缓存目录不存在: {}", args.cache_dir.display());
        println!("   提示: 请先下载模型");
        return Ok(());
    }
    
    // 列出可用模型
    let models = list_available_models(args.cache_dir.to_str().unwrap())?;
    
    if models.is_empty() {
        println!("📭 没有找到已下载的模型");
        println!("   提示: 使用 'fastembed models download --model BGELargeZHV15' 下载模型");
    } else {
        println!("✅ 找到 {} 个已下载的模型:", models.len());
        println!();
        println!("{:<20} {:<40} {:<10}", "变体名", "模型代码", "维度");
        println!("{}", "─".repeat(72));
        
        for model in models {
            println!(
                "{:<20} {:<40} {:<10}",
                model.variant,
                model.code,
                model.dim
            );
        }
    }
    
    Ok(())
}
