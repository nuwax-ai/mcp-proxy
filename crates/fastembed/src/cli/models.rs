use super::{DownloadArgs, ListArgs};
use crate::models::{ModelInfo, list_available_models};
use anyhow::Result;

/// 执行模型下载
pub async fn download_model(args: DownloadArgs) -> Result<()> {
    use fastembed::{InitOptions, TextEmbedding};

    tracing::info!("Start downloading the model...");

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
    println!("📦 Download model:");
    println!("Variant name: {}", model_info.variant);
    println!("Model code: {}", model_info.code);
    println!("Vector dimensions: {}", model_info.dim);
    println!("Cache directory: {}", args.cache_dir.display());
    println!();

    // 初始化模型（会自动下载）
    let mut options = InitOptions::new(model.clone());
    options = options.with_cache_dir(args.cache_dir.clone());
    options = options.with_show_download_progress(args.progress);

    println!("⬇️ Downloading model files...");
    let start = std::time::Instant::now();

    let _embedding = TextEmbedding::try_new(options)?;

    let elapsed = start.elapsed();

    println!();
    println!("✅ Model download completed!");
    println!("Time taken: {:?}", elapsed);
    println!("Cache location: {}", args.cache_dir.display());

    // 验证文件
    println!();
    println!("🔍 Verify model file...");
    let available = list_available_models(args.cache_dir.to_str().unwrap())?;

    if available.iter().any(|m| m.variant == model_info.variant) {
        println!("✅ Model file verification successful!");
    } else {
        println!("⚠️ WARNING: Model file may be incomplete");
    }

    Ok(())
}

/// 列出已下载的模型
pub async fn list_models(args: ListArgs) -> Result<()> {
    use crate::models::list_available_models;

    println!("📋 Query downloaded models...");
    println!("Type: {}", args.r#type);
    println!("Cache directory: {}", args.cache_dir.display());
    println!();

    // 检查缓存目录是否存在
    if !args.cache_dir.exists() {
        println!("⚠️ The cache directory does not exist: {}", args.cache_dir.display());
        println!("Tip: Please download the model first");
        return Ok(());
    }

    // 列出可用模型
    let models = list_available_models(args.cache_dir.to_str().unwrap())?;

    if models.is_empty() {
        println!("📭 No downloaded model found");
        println!("Tip: Use 'fastembed models download --model BGELargeZHV15' to download the model");
    } else {
        println!("✅ Found {} downloaded models:", models.len());
        println!();
        println!("{:<20} {:<40} {:<10}", "Variant", "Model Code", "Dim");
        println!("{}", "─".repeat(72));

        for model in models {
            println!("{:<20} {:<40} {:<10}", model.variant, model.code, model.dim);
        }
    }

    Ok(())
}
