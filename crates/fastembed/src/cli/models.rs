use super::{DownloadArgs, ListArgs};
use crate::models::{EmbeddingType, ModelInfo, get_or_init_model, list_available_models};
use anyhow::Result;
use std::str::FromStr;

/// 执行模型下载
pub async fn download_model(args: DownloadArgs) -> Result<()> {
    tracing::info!("Start downloading the model...");

    let model_type = EmbeddingType::from_str(&args.r#type)?;

    // 解析模型标识，复用 get_or_init_model 的解析+初始化逻辑
    let model_input = if let Some(model_name) = args.model {
        model_name
    } else if let Some(code) = args.code {
        code
    } else {
        anyhow::bail!("必须指定 --model 或 --code 参数");
    };

    // 先取一份 ModelInfo（仅用于展示；目录外模型 dim=0）
    let preview = preview_model_info(model_type, &model_input);

    println!("📦 Download model:");
    println!("Type: {}", model_type);
    println!("Variant name: {}", preview.variant);
    println!("Model code: {}", preview.code);
    println!("Vector dimensions: {}", preview.dim);
    println!("Cache directory: {}", args.cache_dir.display());
    println!();

    println!("⬇️ Downloading model files...");
    let start = std::time::Instant::now();

    // 下载阶段不需要 GPU EP（device=cpu），单实例（pool_size=1）即可触发文件下载与初始化
    let _ = get_or_init_model(
        model_type,
        &model_input,
        Some(args.cache_dir.to_string_lossy().to_string()),
        None,
        "cpu",
        1,
    )?;

    let elapsed = start.elapsed();

    println!();
    println!("✅ Model download completed!");
    println!("Time taken: {:?}", elapsed);
    println!("Cache location: {}", args.cache_dir.display());

    // 验证文件
    println!();
    println!("🔍 Verify model file...");
    let cache_dir_str = args.cache_dir.to_str().unwrap_or_default();
    let available = list_available_models(model_type, cache_dir_str)?;

    if available.iter().any(|m| m.code == preview.code) {
        println!("✅ Model file verification successful!");
    } else {
        println!("⚠️ WARNING: Model file may be incomplete (目录外模型不参与校验)");
    }

    Ok(())
}

/// 列出已下载的模型
pub async fn list_models(args: ListArgs) -> Result<()> {
    let model_type = EmbeddingType::from_str(&args.r#type)?;

    println!("📋 Query downloaded models...");
    println!("Type: {}", model_type);
    println!("Cache directory: {}", args.cache_dir.display());
    println!();

    // 检查缓存目录是否存在
    if !args.cache_dir.exists() {
        println!(
            "⚠️ The cache directory does not exist: {}",
            args.cache_dir.display()
        );
        println!("Tip: Please download the model first");
        return Ok(());
    }

    // 列出可用模型
    let cache_dir_str = args.cache_dir.to_str().unwrap_or_default();
    let models = list_available_models(model_type, cache_dir_str)?;

    if models.is_empty() {
        println!("📭 No downloaded model found");
        println!(
            "Tip: Use 'fastembed models download --type {} --model <name>' to download the model",
            model_type
        );
    } else {
        println!("✅ Found {} downloaded models:", models.len());
        println!();
        println!(
            "{:<8} {:<24} {:<40} {:<8}",
            "Type", "Variant", "Model Code", "Dim"
        );
        println!("{}", "─".repeat(82));

        for model in models {
            println!(
                "{:<8} {:<24} {:<40} {:<8}",
                model.r#type, model.variant, model.code, model.dim
            );
        }
    }

    Ok(())
}

/// 解析模型并构造预览信息（仅展示用）
fn preview_model_info(model_type: EmbeddingType, input: &str) -> ModelInfo {
    // 复用 get_or_init_model 的 resolve 逻辑会触发初始化（下载），
    // 这里仅需展示信息，用 from_catalog 的解析路径即可。
    match crate::models::resolve_code_for_display(model_type, input) {
        Some(code) => ModelInfo::from_catalog(model_type, &code),
        None => ModelInfo::from_catalog(model_type, input),
    }
}
