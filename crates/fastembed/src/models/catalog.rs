use anyhow::Result;
use fastembed::{ImageEmbedding, SparseTextEmbedding, TextEmbedding};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};

use super::EmbeddingType;

/// Model catalog entry: variant name, HuggingFace repo, embedding dimension
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub variant: String,
    pub code: String,
    pub dim: usize,
}

impl ModelEntry {
    fn matches(&self, input: &str) -> bool {
        self.variant == input || self.code.eq_ignore_ascii_case(input)
    }
}

static TEXT_CATALOG: Lazy<Vec<ModelEntry>> = Lazy::new(|| {
    TextEmbedding::list_supported_models()
        .into_iter()
        .map(|info| ModelEntry {
            variant: format!("{:?}", info.model),
            code: info.model_code,
            dim: info.dim,
        })
        .collect()
});

static IMAGE_CATALOG: Lazy<Vec<ModelEntry>> = Lazy::new(|| {
    ImageEmbedding::list_supported_models()
        .into_iter()
        .map(|info| ModelEntry {
            variant: format!("{:?}", info.model),
            code: info.model_code,
            dim: info.dim,
        })
        .collect()
});

static SPARSE_CATALOG: Lazy<Vec<ModelEntry>> = Lazy::new(|| {
    SparseTextEmbedding::list_supported_models()
        .into_iter()
        .map(|info| ModelEntry {
            variant: format!("{:?}", info.model),
            code: info.model_code,
            dim: info.dim,
        })
        .collect()
});

pub(crate) fn catalog_for(t: EmbeddingType) -> &'static [ModelEntry] {
    match t {
        EmbeddingType::Text => &TEXT_CATALOG,
        EmbeddingType::Image => &IMAGE_CATALOG,
        EmbeddingType::Sparse => &SPARSE_CATALOG,
    }
}

pub(crate) fn lookup_entry(t: EmbeddingType, input: &str) -> Option<&ModelEntry> {
    catalog_for(t).iter().find(|e| e.matches(input))
}

/// Model info for API responses
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ModelInfo {
    #[schema(example = "text")]
    pub r#type: String,
    #[schema(example = "BGELargeZHV15")]
    pub variant: String,
    #[schema(example = "Xenova/bge-large-zh-v1.5")]
    pub code: String,
    #[schema(example = 1024)]
    pub dim: usize,
}

impl ModelInfo {
    pub fn from_catalog(model_type: EmbeddingType, code: &str) -> Self {
        let entry = catalog_for(model_type)
            .iter()
            .find(|e| e.code.eq_ignore_ascii_case(code));
        match entry {
            Some(e) => Self {
                r#type: model_type.to_string(),
                variant: e.variant.to_string(),
                code: e.code.to_string(),
                dim: e.dim,
            },
            None => Self {
                r#type: model_type.to_string(),
                variant: code.to_string(),
                code: code.to_string(),
                dim: 0,
            },
        }
    }
}

/// List locally cached models (offline check only)
pub fn list_available_models(model_type: EmbeddingType, cache_dir: &str) -> Result<Vec<ModelInfo>> {
    let cache_path = PathBuf::from(cache_dir);
    if !cache_path.exists() {
        return Ok(vec![]);
    }
    let available: Vec<ModelInfo> = catalog_for(model_type)
        .iter()
        .filter(|entry| check_model_files_exist(&cache_path, entry))
        .cloned()
        .map(|entry| ModelInfo {
            r#type: model_type.to_string(),
            variant: entry.variant,
            code: entry.code,
            dim: entry.dim,
        })
        .collect();
    Ok(available)
}

/// Check if model files exist in hf-hub cache structure
pub(crate) fn check_model_files_exist(cache_path: &Path, entry: &ModelEntry) -> bool {
    let model_dir_name = format!("models--{}", entry.code.replace('/', "--"));
    let model_dir = cache_path.join(&model_dir_name);

    if !model_dir.exists() || !model_dir.is_dir() {
        return false;
    }
    let snapshots = model_dir.join("snapshots");
    if !snapshots.is_dir() {
        return false;
    }
    let snapshots_iter = match std::fs::read_dir(&snapshots) {
        Ok(it) => it,
        Err(_) => return false,
    };
    for entry_res in snapshots_iter.flatten() {
        let snapshot_dir = entry_res.path();
        if !snapshot_dir.is_dir() {
            continue;
        }
        if dir_has_onnx(&snapshot_dir) {
            return true;
        }
    }
    false
}

/// Recursively check if directory contains .onnx file
pub(crate) fn dir_has_onnx(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry_res in entries.flatten() {
        let path = entry_res.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("onnx") {
            return true;
        }
        if path.is_dir() && dir_has_onnx(&path) {
            return true;
        }
    }
    false
}
