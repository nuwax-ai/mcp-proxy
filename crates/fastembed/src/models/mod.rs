pub mod catalog;
pub mod download;
pub mod pool;

use crate::config::Device;
use anyhow::{Context, Result, anyhow};
use fastembed::{
    EmbeddingModel, ExecutionProviderDispatch, ImageEmbedding, ImageEmbeddingModel,
    ImageInitOptions, SparseEmbedding, SparseInitOptions, SparseModel, SparseTextEmbedding,
    TextEmbedding, TextInitOptions,
};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

// Re-exports
pub use catalog::{ModelInfo, list_available_models};
pub use download::download_model_from_url;
pub use pool::{CacheKey, INIT_LOCKS, MODEL_CACHE, ModelPool};

/// Embedding type
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingType {
    Text,
    Image,
    Sparse,
}

impl EmbeddingType {
    pub fn as_str(self) -> &'static str {
        match self {
            EmbeddingType::Text => "text",
            EmbeddingType::Image => "image",
            EmbeddingType::Sparse => "sparse",
        }
    }
}

impl std::fmt::Display for EmbeddingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EmbeddingType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "text" => Ok(EmbeddingType::Text),
            "image" => Ok(EmbeddingType::Image),
            "sparse" => Ok(EmbeddingType::Sparse),
            other => Err(anyhow!(
                "unknown embedding type: {} (text/image/sparse)",
                other
            )),
        }
    }
}

/// Initialized model (hides fastembed's three engine types)
pub enum InitializedModel {
    Text(TextEmbedding),
    Image(ImageEmbedding),
    Sparse(SparseTextEmbedding),
}

/// Embedding output: dense (text/image) or sparse
pub enum EmbedOutput {
    Dense(Vec<Vec<f32>>),
    Sparse(Vec<SparseEmbedding>),
}

impl InitializedModel {
    pub fn embed(&mut self, inputs: Vec<String>, batch_size: Option<usize>) -> Result<EmbedOutput> {
        match self {
            InitializedModel::Text(e) => {
                let out = e.embed(inputs, batch_size)?;
                Ok(EmbedOutput::Dense(out))
            }
            InitializedModel::Image(e) => {
                let out = e.embed(inputs, batch_size)?;
                Ok(EmbedOutput::Dense(out))
            }
            InitializedModel::Sparse(e) => {
                let out = e.embed(inputs, batch_size)?;
                Ok(EmbedOutput::Sparse(out))
            }
        }
    }
}

// ── Resolution ────────────────────────────────────────────────────────────────

/// Resolved model: typed model enum + normalized code, computed once then reused
#[derive(Clone)]
enum Resolved {
    Text(EmbeddingModel, String),
    Image(ImageEmbeddingModel, String),
    Sparse(SparseModel, String),
}

impl Resolved {
    fn code(&self) -> &str {
        match self {
            Resolved::Text(_, c) | Resolved::Image(_, c) | Resolved::Sparse(_, c) => c,
        }
    }

    fn init(
        self,
        cache_dir: Option<String>,
        max_length: Option<usize>,
        eps: Vec<ExecutionProviderDispatch>,
        show_progress: bool,
    ) -> Result<InitializedModel> {
        match self {
            Resolved::Text(m, _) => {
                init_text(m, cache_dir, max_length, eps, show_progress).map(InitializedModel::Text)
            }
            Resolved::Image(m, _) => {
                init_image(m, cache_dir, eps, show_progress).map(InitializedModel::Image)
            }
            Resolved::Sparse(m, _) => init_sparse(m, cache_dir, max_length, eps, show_progress)
                .map(InitializedModel::Sparse),
        }
    }
}

/// Resolve variant name or model code to typed model + normalized code.
/// catalog lookup maps codes to variant names (FromStr only accepts variants for Text,
/// codes for Image/Sparse).
fn resolve(t: EmbeddingType, input: &str) -> Result<Resolved> {
    let catalog_entry = catalog::lookup_entry(t, input);
    let variant = catalog_entry.map(|e| e.variant.as_str()).unwrap_or(input);
    let code = catalog_entry
        .map(|e| e.code.clone())
        .unwrap_or_else(|| input.to_string());

    match t {
        EmbeddingType::Text => EmbeddingModel::from_str(variant)
            .map(|m| Resolved::Text(m, code))
            .map_err(|_| anyhow!("unknown text model: {}", input)),
        EmbeddingType::Image => ImageEmbeddingModel::from_str(&code)
            .map(|m| Resolved::Image(m, code))
            .map_err(|_| anyhow!("unknown image model: {}", input)),
        EmbeddingType::Sparse => SparseModel::from_str(&code)
            .map(|m| Resolved::Sparse(m, code))
            .map_err(|_| anyhow!("unknown sparse model: {}", input)),
    }
}

pub(crate) fn resolve_code_for_display(t: EmbeddingType, input: &str) -> Option<String> {
    resolve(t, input).ok().map(|r| r.code().to_string())
}

// ── GPU execution providers ───────────────────────────────────────────────────

pub fn resolve_execution_providers(device: Device) -> Vec<ExecutionProviderDispatch> {
    match device {
        Device::Cpu => vec![],
        Device::CoreML => vec![ort::ep::CoreML::default().into()],
        Device::Cuda => vec![ort::ep::CUDA::default().into()],
        Device::DirectML => vec![ort::ep::DirectML::default().into()],
        Device::Auto => resolve_auto_providers(),
    }
}

fn resolve_auto_providers() -> Vec<ExecutionProviderDispatch> {
    let (label, eps): (&str, Vec<ExecutionProviderDispatch>) = {
        #[cfg(target_os = "macos")]
        {
            (
                "CoreML (macOS Apple GPU)",
                vec![ort::ep::CoreML::default().into()],
            )
        }
        #[cfg(target_os = "linux")]
        {
            (
                "CUDA (Linux NVIDIA GPU)",
                vec![ort::ep::CUDA::default().into()],
            )
        }
        #[cfg(target_os = "windows")]
        {
            (
                "DirectML (Windows GPU)",
                vec![ort::ep::DirectML::default().into()],
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            ("CPU (unknown platform)", vec![])
        }
    };
    tracing::info!("auto -> {}", label);
    eps
}

// ── Model initialization ──────────────────────────────────────────────────────

/// Get or initialize a model, returning (instance pool, model info).
/// Uses double-checked locking with per-model locks.
pub fn get_or_init_model(
    model_type: EmbeddingType,
    model_input: &str,
    cache_dir: Option<String>,
    max_length: Option<usize>,
    device: Device,
    pool_size: usize,
    show_progress: bool,
) -> Result<(Arc<ModelPool<InitializedModel>>, ModelInfo)> {
    let pool_size = pool_size.max(1);
    let resolved = resolve(model_type, model_input)?;
    let code = resolved.code().to_string();
    let cache_key: CacheKey = (model_type, code.clone());

    // Fast path: cache hit
    if let Some(existing) = MODEL_CACHE.get(&cache_key) {
        tracing::debug!("model cache hit: {:?}", cache_key);
        return Ok((existing.clone(), ModelInfo::from_catalog(model_type, &code)));
    }

    // Slow path: per-model lock
    let lock = INIT_LOCKS
        .entry(cache_key.clone())
        .or_insert_with(|| Arc::new(std::sync::Mutex::new(())))
        .clone();
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    // Double-check
    if let Some(existing) = MODEL_CACHE.get(&cache_key) {
        tracing::debug!("model cache hit (post-lock): {:?}", cache_key);
        return Ok((existing.clone(), ModelInfo::from_catalog(model_type, &code)));
    }

    tracing::info!(
        "init model: {:?}, device: {}, pool_size: {}",
        cache_key,
        device,
        pool_size
    );
    let mut instances = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let eps = resolve_execution_providers(device);
        instances.push(
            resolved
                .clone()
                .init(cache_dir.clone(), max_length, eps, show_progress)?,
        );
        tracing::debug!("pool instance {}/{} ready", i + 1, pool_size);
    }
    let pool = Arc::new(ModelPool::new(instances));
    MODEL_CACHE.insert(cache_key.clone(), pool.clone());
    INIT_LOCKS.remove(&cache_key);

    tracing::info!(
        "model init ok: {:?} ({} instance(s))",
        cache_key,
        pool.len()
    );
    Ok((pool, ModelInfo::from_catalog(model_type, &code)))
}

fn init_text(
    model: EmbeddingModel,
    cache_dir: Option<String>,
    max_length: Option<usize>,
    eps: Vec<ExecutionProviderDispatch>,
    show_progress: bool,
) -> Result<TextEmbedding> {
    let mut options = TextInitOptions::new(model.clone());
    if let Some(dir) = cache_dir {
        options = options.with_cache_dir(PathBuf::from(dir));
    }
    if let Some(len) = max_length {
        options = options.with_max_length(len);
    }
    if !eps.is_empty() {
        options = options.with_execution_providers(eps);
    }
    if show_progress {
        options = options.with_show_download_progress(true);
    }
    TextEmbedding::try_new(options)
        .with_context(|| format!("failed to init text model: {:?}", model))
}

fn init_image(
    model: ImageEmbeddingModel,
    cache_dir: Option<String>,
    eps: Vec<ExecutionProviderDispatch>,
    show_progress: bool,
) -> Result<ImageEmbedding> {
    let mut options = ImageInitOptions::new(model.clone());
    if let Some(dir) = cache_dir {
        options = options.with_cache_dir(PathBuf::from(dir));
    }
    if !eps.is_empty() {
        options = options.with_execution_providers(eps);
    }
    if show_progress {
        options = options.with_show_download_progress(true);
    }
    ImageEmbedding::try_new(options)
        .with_context(|| format!("failed to init image model: {:?}", model))
}

fn init_sparse(
    model: SparseModel,
    cache_dir: Option<String>,
    max_length: Option<usize>,
    eps: Vec<ExecutionProviderDispatch>,
    show_progress: bool,
) -> Result<SparseTextEmbedding> {
    let mut options = SparseInitOptions::new(model.clone());
    if let Some(dir) = cache_dir {
        options = options.with_cache_dir(PathBuf::from(dir));
    }
    if let Some(len) = max_length {
        options = options.with_max_length(len);
    }
    if !eps.is_empty() {
        options = options.with_execution_providers(eps);
    }
    if show_progress {
        options = options.with_show_download_progress(true);
    }
    SparseTextEmbedding::try_new(options)
        .with_context(|| format!("failed to init sparse model: {:?}", model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_type_from_str_valid() {
        assert_eq!(
            EmbeddingType::from_str("text").unwrap(),
            EmbeddingType::Text
        );
        assert_eq!(
            EmbeddingType::from_str("image").unwrap(),
            EmbeddingType::Image
        );
        assert_eq!(
            EmbeddingType::from_str("sparse").unwrap(),
            EmbeddingType::Sparse
        );
        assert_eq!(
            EmbeddingType::from_str("TEXT").unwrap(),
            EmbeddingType::Text
        );
        assert_eq!(
            EmbeddingType::from_str("Image").unwrap(),
            EmbeddingType::Image
        );
    }

    #[test]
    fn embedding_type_from_str_invalid() {
        assert!(EmbeddingType::from_str("video").is_err());
    }

    #[test]
    fn embedding_type_serde_lowercase() {
        let t: EmbeddingType = serde_json::from_str("\"text\"").unwrap();
        assert_eq!(t, EmbeddingType::Text);
        let json = serde_json::to_string(&EmbeddingType::Image).unwrap();
        assert_eq!(json, "\"image\"");
    }

    #[test]
    fn embedding_type_display() {
        assert_eq!(EmbeddingType::Text.to_string(), "text");
        assert_eq!(EmbeddingType::Image.to_string(), "image");
        assert_eq!(EmbeddingType::Sparse.to_string(), "sparse");
    }

    #[test]
    fn resolve_text_variant_and_code() {
        match resolve(EmbeddingType::Text, "AllMiniLML6V2").unwrap() {
            Resolved::Text(m, code) => {
                assert_eq!(m, EmbeddingModel::AllMiniLML6V2);
                assert_eq!(code, "Qdrant/all-MiniLM-L6-v2-onnx");
            }
            other => panic!("expected Text, got {:?}", other.code()),
        }
        match resolve(EmbeddingType::Text, "Qdrant/all-MiniLM-L6-v2-onnx").unwrap() {
            Resolved::Text(m, code) => {
                assert_eq!(m, EmbeddingModel::AllMiniLML6V2);
                assert_eq!(code, "Qdrant/all-MiniLM-L6-v2-onnx");
            }
            other => panic!("expected Text, got {:?}", other.code()),
        }
        match resolve(EmbeddingType::Text, "Xenova/bge-large-zh-v1.5").unwrap() {
            Resolved::Text(m, _) => assert_eq!(m, EmbeddingModel::BGELargeZHV15),
            other => panic!("expected Text, got {:?}", other.code()),
        }
    }

    #[test]
    fn resolve_text_unknown() {
        assert!(resolve(EmbeddingType::Text, "not-a-real-model").is_err());
    }

    #[test]
    fn resolve_image_variant_and_code() {
        match resolve(EmbeddingType::Image, "ClipVitB32").unwrap() {
            Resolved::Image(m, code) => {
                assert_eq!(m, ImageEmbeddingModel::ClipVitB32);
                assert_eq!(code, "Qdrant/clip-ViT-B-32-vision");
            }
            other => panic!("expected Image, got {:?}", other.code()),
        }
        match resolve(EmbeddingType::Image, "Qdrant/clip-ViT-B-32-vision").unwrap() {
            Resolved::Image(m, _) => assert_eq!(m, ImageEmbeddingModel::ClipVitB32),
            other => panic!("expected Image, got {:?}", other.code()),
        }
        assert!(resolve(EmbeddingType::Image, "nope").is_err());
    }

    #[test]
    fn resolve_sparse_variant_and_code() {
        match resolve(EmbeddingType::Sparse, "SPLADEPPV1").unwrap() {
            Resolved::Sparse(m, code) => {
                assert_eq!(m, SparseModel::SPLADEPPV1);
                assert_eq!(code, "Qdrant/Splade_PP_en_v1");
            }
            other => panic!("expected Sparse, got {:?}", other.code()),
        }
        match resolve(EmbeddingType::Sparse, "BAAI/bge-m3").unwrap() {
            Resolved::Sparse(m, _) => assert_eq!(m, SparseModel::BGEM3),
            other => panic!("expected Sparse, got {:?}", other.code()),
        }
        assert!(resolve(EmbeddingType::Sparse, "nope").is_err());
    }

    #[test]
    fn resolve_code_catalog_canonicalizes_variant() {
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "AllMiniLML6V2"),
            Some("Qdrant/all-MiniLM-L6-v2-onnx".to_string())
        );
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "Xenova/bge-large-zh-v1.5"),
            Some("Xenova/bge-large-zh-v1.5".to_string())
        );
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "nonsense-xyz"),
            None
        );
    }

    #[test]
    fn model_info_from_catalog_curated() {
        let info = ModelInfo::from_catalog(EmbeddingType::Text, "Xenova/bge-large-zh-v1.5");
        assert_eq!(info.r#type, "text");
        assert_eq!(info.variant, "BGELargeZHV15");
        assert_eq!(info.code, "Xenova/bge-large-zh-v1.5");
        assert_eq!(info.dim, 1024);
    }

    #[test]
    fn model_info_from_catalog_fallback() {
        let info = ModelInfo::from_catalog(EmbeddingType::Text, "some-unknown/code");
        assert_eq!(info.variant, "some-unknown/code");
        assert_eq!(info.dim, 0);
    }

    #[test]
    fn execution_providers_mapping() {
        assert!(resolve_execution_providers(Device::Cpu).is_empty());
        assert!(!resolve_execution_providers(Device::CoreML).is_empty());
        assert!(!resolve_execution_providers(Device::Cuda).is_empty());
        assert!(!resolve_execution_providers(Device::DirectML).is_empty());
        let _auto_eps = resolve_execution_providers(Device::Auto);
    }

    fn entry(code: &str) -> catalog::ModelEntry {
        catalog::ModelEntry {
            variant: "TEST".to_string(),
            code: code.to_string(),
            dim: 0,
        }
    }

    #[test]
    fn check_model_files_detects_onnx() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let model_dir = cache.join("models--ns--repo");
        let snap = model_dir.join("snapshots").join("abc");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("model.onnx"), b"").unwrap();
        let e = entry("ns/repo");
        assert!(catalog::check_model_files_exist(cache, &e));
    }

    #[test]
    fn check_model_files_missing_onnx() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let model_dir = cache.join("models--ns--repo");
        let snap = model_dir.join("snapshots").join("abc");
        std::fs::create_dir_all(&snap).unwrap();
        let e = entry("ns/repo");
        assert!(!catalog::check_model_files_exist(cache, &e));
    }

    #[test]
    fn check_model_files_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let e = entry("ns/repo");
        assert!(!catalog::check_model_files_exist(cache, &e));
    }

    #[test]
    fn list_available_models_empty_when_no_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let models = list_available_models(EmbeddingType::Text, missing.to_str().unwrap()).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn list_available_models_finds_present() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let model_dir = cache.join("models--Xenova--bge-large-zh-v1.5");
        let snap = model_dir.join("snapshots").join("abc");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("model.onnx"), b"").unwrap();
        let models = list_available_models(EmbeddingType::Text, cache.to_str().unwrap()).unwrap();
        assert!(models.iter().any(|m| m.code.contains("bge-large-zh")));
    }

    #[test]
    fn model_pool_round_robin() {
        let pool = ModelPool::new(vec![1i32, 2, 3]);
        assert_eq!(pool.len(), 3);
        let first = *pool.pick().lock().unwrap();
        let second = *pool.pick().lock().unwrap();
        let third = *pool.pick().lock().unwrap();
        let fourth = *pool.pick().lock().unwrap();
        assert_eq!((first, second, third, fourth), (1, 2, 3, 1));
    }

    #[test]
    #[should_panic(expected = "remainder")]
    fn model_pool_empty_panics() {
        let pool: ModelPool<()> = ModelPool::new(vec![]);
        pool.pick();
    }

    #[test]
    fn extract_tar_gz_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("models--test--model");
        let snapshots = src_dir.join("snapshots").join("abc123");
        std::fs::create_dir_all(&snapshots).unwrap();
        std::fs::write(snapshots.join("model.onnx"), b"fake onnx").unwrap();
        std::fs::write(snapshots.join("tokenizer.json"), b"{}").unwrap();

        let archive_path = tmp.path().join("model.tar.gz");
        {
            let file = std::fs::File::create(&archive_path).unwrap();
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            let mut ar = tar::Builder::new(enc);
            ar.append_dir_all("models--test--model", &src_dir).unwrap();
            ar.finish().unwrap();
        }

        let dest = tmp.path().join("cache");
        download::extract_tar_gz(&archive_path, &dest).unwrap();

        assert!(
            dest.join("models--test--model/snapshots/abc123/model.onnx")
                .exists()
        );
        assert!(
            dest.join("models--test--model/snapshots/abc123/tokenizer.json")
                .exists()
        );
    }

    #[test]
    fn dir_has_onnx_deeply_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("model.onnx"), b"").unwrap();
        assert!(catalog::dir_has_onnx(tmp.path()));

        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(!catalog::dir_has_onnx(&empty));
    }

    #[test]
    #[ignore = "requires network to download AllMiniLML6V2 model"]
    fn embed_smoke_text() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().to_str().unwrap().to_string();
        let (pool, info) = get_or_init_model(
            EmbeddingType::Text,
            "AllMiniLML6V2",
            Some(cache.clone()),
            None,
            Device::Cpu,
            1,
            true,
        )
        .expect("model init failed (check network)");

        assert_eq!(info.dim, 384);
        assert_eq!(pool.len(), 1);
        let instance = pool.pick();
        let mut guard = instance.lock().unwrap();
        let out = guard
            .embed(vec!["hello world".to_string()], Some(1))
            .expect("embed failed");
        match out {
            EmbedOutput::Dense(vec) => {
                assert_eq!(vec.len(), 1);
                assert_eq!(vec[0].len(), 384);
            }
            EmbedOutput::Sparse(_) => panic!("text model should return dense vectors"),
        }
        drop(guard);

        let listed = list_available_models(EmbeddingType::Text, &cache).unwrap();
        assert!(
            listed.iter().any(|m| m.variant == "AllMiniLML6V2"),
            "list_available_models did not find AllMiniLML6V2: {:?}",
            listed
        );
    }
}
