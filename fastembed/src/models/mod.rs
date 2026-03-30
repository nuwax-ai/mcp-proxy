use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use once_cell::sync::Lazy;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

/// 全局模型缓存
pub static MODEL_CACHE: Lazy<DashMap<EmbeddingModel, Arc<Mutex<TextEmbedding>>>> =
    Lazy::new(DashMap::new);

/// 解析模型标识（支持变体名和模型代码）
pub fn parse_model(user_input: &str) -> Result<EmbeddingModel> {
    // 尝试直接匹配变体名
    match user_input {
        "BGELargeZHV15" => Ok(EmbeddingModel::BGELargeZHV15),
        "BGESmallZHV15" => Ok(EmbeddingModel::BGESmallZHV15),
        "BGEBaseENV15" => Ok(EmbeddingModel::BGEBaseENV15),
        "BGESmallENV15" => Ok(EmbeddingModel::BGESmallENV15),
        "BGELargeENV15" => Ok(EmbeddingModel::BGELargeENV15),
        "AllMiniLML6V2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "AllMiniLML12V2" => Ok(EmbeddingModel::AllMiniLML12V2),
        // 如果不是变体名，尝试使用 FromStr 解析模型代码
        other => EmbeddingModel::from_str(other).map_err(|_| anyhow!("未知模型: {}", other)),
    }
}

/// 获取或初始化模型
pub fn get_or_init_model(
    model: EmbeddingModel,
    cache_dir: Option<String>,
    max_length: Option<usize>,
) -> Result<Arc<Mutex<TextEmbedding>>> {
    // 检查缓存
    if let Some(existing) = MODEL_CACHE.get(&model) {
        tracing::debug!("Get model from cache: {:?}", model);
        return Ok(existing.clone());
    }

    // 初始化模型
    tracing::info!("Initialization model: {:?}", model);
    let mut options = InitOptions::new(model.clone());

    if let Some(dir) = cache_dir {
        options = options.with_cache_dir(PathBuf::from(dir));
    }

    if let Some(len) = max_length {
        options = options.with_max_length(len);
    }

    // 显示下载进度
    options = options.with_show_download_progress(true);

    let embedding =
        TextEmbedding::try_new(options).with_context(|| format!("无法初始化模型: {:?}", model))?;

    let arc = Arc::new(Mutex::new(embedding));
    let model_key = model.clone();
    MODEL_CACHE.insert(model_key, arc.clone());

    tracing::info!("Model initialization successful: {:?}", model);
    Ok(arc)
}

/// 模型信息
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ModelInfo {
    /// 模型变体名称
    #[schema(example = "BGELargeZHV15")]
    pub variant: String,

    /// 模型代码（Hugging Face 仓库）
    #[schema(example = "Xenova/bge-large-zh-v1.5")]
    pub code: String,

    /// 向量维度
    #[schema(example = 1024)]
    pub dim: usize,
}

impl ModelInfo {
    pub fn from_embedding_model(model: &EmbeddingModel) -> Self {
        let (variant, code, dim) = match model {
            EmbeddingModel::BGELargeZHV15 => ("BGELargeZHV15", "Xenova/bge-large-zh-v1.5", 1024),
            EmbeddingModel::BGESmallZHV15 => ("BGESmallZHV15", "Xenova/bge-small-zh-v1.5", 512),
            EmbeddingModel::BGEBaseENV15 => ("BGEBaseENV15", "Xenova/bge-base-en-v1.5", 768),
            EmbeddingModel::BGESmallENV15 => ("BGESmallENV15", "Xenova/bge-small-en-v1.5", 384),
            EmbeddingModel::BGELargeENV15 => ("BGELargeENV15", "Xenova/bge-large-en-v1.5", 1024),
            EmbeddingModel::AllMiniLML6V2 => (
                "AllMiniLML6V2",
                "sentence-transformers/all-MiniLM-L6-v2",
                384,
            ),
            EmbeddingModel::AllMiniLML12V2 => (
                "AllMiniLML12V2",
                "sentence-transformers/all-MiniLM-L12-v2",
                384,
            ),
            _ => ("Unknown", "unknown", 0),
        };

        Self {
            variant: variant.to_string(),
            code: code.to_string(),
            dim,
        }
    }
}

/// 列出本地已下载的模型（仅离线检查）
pub fn list_available_models(cache_dir: &str) -> Result<Vec<ModelInfo>> {
    let cache_path = PathBuf::from(cache_dir);

    // 如果缓存目录不存在，返回空列表
    if !cache_path.exists() {
        return Ok(vec![]);
    }

    let all_models = vec![
        EmbeddingModel::BGELargeZHV15,
        EmbeddingModel::BGESmallZHV15,
        EmbeddingModel::BGEBaseENV15,
        EmbeddingModel::BGESmallENV15,
        EmbeddingModel::BGELargeENV15,
        EmbeddingModel::AllMiniLML6V2,
        EmbeddingModel::AllMiniLML12V2,
    ];

    let available: Vec<ModelInfo> = all_models
        .into_iter()
        .filter(|model| check_model_files_exist(&cache_path, model))
        .map(|model| ModelInfo::from_embedding_model(&model))
        .collect();

    Ok(available)
}

/// 检查模型文件是否存在（简化版本）
fn check_model_files_exist(cache_path: &PathBuf, model: &EmbeddingModel) -> bool {
    // 这是一个简化实现
    // fastembed 使用 hf-hub 的缓存结构
    // 例如 "Xenova/bge-large-zh-v1.5" -> "models--Xenova--bge-large-zh-v1.5"

    let model_info = ModelInfo::from_embedding_model(model);
    let model_code = model_info.code;

    // 从模型代码转换为 hf-hub 缓存目录名
    // "Xenova/bge-large-zh-v1.5" -> "models--Xenova--bge-large-zh-v1.5"
    let model_dir_name = format!("models--{}", model_code.replace('/', "--"));
    let model_dir = cache_path.join(&model_dir_name);

    // 检查目录是否存在且不为空
    if model_dir.exists() && model_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&model_dir) {
            return entries.count() > 0;
        }
    }

    false
}
