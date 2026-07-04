use anyhow::{Context, Result, anyhow};
use dashmap::DashMap;
use fastembed::{
    EmbeddingModel, ExecutionProviderDispatch, ImageEmbedding, ImageEmbeddingModel,
    ImageInitOptions, SparseEmbedding, SparseInitOptions, SparseModel, SparseTextEmbedding,
    TextEmbedding, TextInitOptions,
};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 单个模型实例（包在 Mutex 里：fastembed embed 需 &mut self）
type Instance = Arc<Mutex<InitializedModel>>;

/// 模型实例池：N 个独立实例，round-robin 分配。
/// - pool_size=1：退化为单实例，并发请求排队（CPU 推理下通常最优，避免线程超订阅）
/// - pool_size>1：允许 N 路并发推理（代价 N× 内存，每个实例独立加载 ONNX 会话）
pub struct ModelPool {
    instances: Vec<Instance>,
    next: AtomicUsize,
}

impl ModelPool {
    fn new(instances: Vec<InitializedModel>) -> Self {
        let instances = instances
            .into_iter()
            .map(|m| Arc::new(Mutex::new(m)))
            .collect();
        Self {
            instances,
            next: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// round-robin 取一个实例。并发请求被分散到不同实例 → 最多 N 路并行；
    /// 若恰好命中同一实例则在该实例上排队。
    pub fn pick(&self) -> Instance {
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % self.instances.len();
        self.instances[idx].clone()
    }
}

/// 全局模型缓存：按 (类型, 模型代码) 索引；每项是 N 实例池
pub static MODEL_CACHE: Lazy<DashMap<(EmbeddingType, String), Arc<ModelPool>>> =
    Lazy::new(DashMap::new);

/// 初始化串行锁：用于 double-checked locking。
/// 仅在「缓存未命中 → 初始化」慢路径上持有，序列化同一模型的并发首次加载，
/// 避免 ort 在并发初始化同一缓存模型时互相冲突而失败。
/// 快路径（命中缓存）不触碰此锁，运行时 embed 不受影响。
static INIT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 嵌入类型
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingType {
    /// 文本嵌入（稠密）
    Text,
    /// 图像嵌入（稠密）
    Image,
    /// 稀疏文本嵌入（SPLADE/BGE-M3）
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
            other => Err(anyhow!("未知嵌入类型: {}（支持 text/image/sparse）", other)),
        }
    }
}

/// 已初始化的模型（屏蔽 fastembed 三种引擎的差异）
pub enum InitializedModel {
    Text(TextEmbedding),
    Image(ImageEmbedding),
    Sparse(SparseTextEmbedding),
}

/// 嵌入计算结果：稠密（text/image）或稀疏（sparse）
pub enum EmbedOutput {
    /// 稠密向量（text / image）
    Dense(Vec<Vec<f32>>),
    /// 稀疏向量（sparse）
    Sparse(Vec<SparseEmbedding>),
}

impl InitializedModel {
    /// 执行嵌入：text/image 输入为文本/路径，sparse 输入为文本
    pub fn embed(&mut self, inputs: Vec<String>, batch_size: Option<usize>) -> Result<EmbedOutput> {
        match self {
            // fastembed::Embedding 是 Vec<f32> 的类型别名，故 out 即 Vec<Vec<f32>>
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

// ────────────────────────────────────────────────────────────────────────────
// 模型目录（变体名 / 模型代码 / 维度）
// 维度信息仅用于响应展示；解析仍走 fastembed 的 FromStr，支持目录外的模型（dim=0）
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
struct ModelEntry {
    variant: &'static str,
    code: &'static str,
    dim: usize,
}

impl ModelEntry {
    fn matches(&self, input: &str) -> bool {
        self.variant == input || self.code.eq_ignore_ascii_case(input)
    }
}

const TEXT_CATALOG: &[ModelEntry] = &[
    ModelEntry {
        variant: "BGELargeZHV15",
        code: "Xenova/bge-large-zh-v1.5",
        dim: 1024,
    },
    ModelEntry {
        variant: "BGESmallZHV15",
        code: "Xenova/bge-small-zh-v1.5",
        dim: 512,
    },
    ModelEntry {
        variant: "BGEBaseENV15",
        code: "Xenova/bge-base-en-v1.5",
        dim: 768,
    },
    ModelEntry {
        variant: "BGESmallENV15",
        code: "Xenova/bge-small-en-v1.5",
        dim: 384,
    },
    ModelEntry {
        variant: "BGELargeENV15",
        code: "Xenova/bge-large-en-v1.5",
        dim: 1024,
    },
    ModelEntry {
        variant: "AllMiniLML6V2",
        code: "Qdrant/all-MiniLM-L6-v2-onnx",
        dim: 384,
    },
    ModelEntry {
        variant: "AllMiniLML12V2",
        code: "Xenova/all-MiniLM-L12-v2",
        dim: 384,
    },
];

const IMAGE_CATALOG: &[ModelEntry] = &[
    ModelEntry {
        variant: "ClipVitB32",
        code: "Qdrant/clip-ViT-B-32-vision",
        dim: 512,
    },
    ModelEntry {
        variant: "Resnet50",
        code: "Qdrant/resnet50-onnx",
        dim: 2048,
    },
    ModelEntry {
        variant: "UnicomVitB16",
        code: "Qdrant/Unicom-ViT-B-16",
        dim: 768,
    },
    ModelEntry {
        variant: "UnicomVitB32",
        code: "Qdrant/Unicom-ViT-B-32",
        dim: 512,
    },
    ModelEntry {
        variant: "NomicEmbedVisionV15",
        code: "nomic-ai/nomic-embed-vision-v1.5",
        dim: 768,
    },
];

const SPARSE_CATALOG: &[ModelEntry] = &[
    ModelEntry {
        variant: "SPLADEPPV1",
        code: "Qdrant/Splade_PP_en_v1",
        dim: 0,
    },
    ModelEntry {
        variant: "BGEM3",
        code: "BAAI/bge-m3",
        dim: 0,
    },
];

fn catalog_for(t: EmbeddingType) -> &'static [ModelEntry] {
    match t {
        EmbeddingType::Text => TEXT_CATALOG,
        EmbeddingType::Image => IMAGE_CATALOG,
        EmbeddingType::Sparse => SPARSE_CATALOG,
    }
}

/// 在目录中查找（按变体名或模型代码，代码大小写不敏感）
fn lookup_entry(t: EmbeddingType, input: &str) -> Option<ModelEntry> {
    catalog_for(t).iter().copied().find(|e| e.matches(input))
}

/// 解析结果：携带类型化模型 + 规范化代码。
/// 一次解析同时产出两者，避免 get_or_init_model 里「取 code」与「初始化」重复解析。
/// Clone 用于构建 N 实例池时复用同一解析结果。
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

    /// 用解析好的模型执行初始化（不再重复解析）
    /// `show_progress` 控制是否打印下载进度（CLI 下载用 true；server 请求触发的懒加载用 false，避免污染日志）
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
            // ImageInitOptions 不支持 max_length
            Resolved::Image(m, _) => {
                init_image(m, cache_dir, eps, show_progress).map(InitializedModel::Image)
            }
            Resolved::Sparse(m, _) => init_sparse(m, cache_dir, max_length, eps, show_progress)
                .map(InitializedModel::Sparse),
        }
    }
}

/// 一次解析：变体名或模型代码 → 类型化模型 + 规范化代码。
/// 命中目录 → 变体映射 + 目录 code；目录外 → fastembed FromStr 校验，代码即输入本身。
fn resolve(t: EmbeddingType, input: &str) -> Result<Resolved> {
    if let Some(entry) = lookup_entry(t, input) {
        return match t {
            EmbeddingType::Text => {
                let m = match entry.variant {
                    "BGELargeZHV15" => EmbeddingModel::BGELargeZHV15,
                    "BGESmallZHV15" => EmbeddingModel::BGESmallZHV15,
                    "BGEBaseENV15" => EmbeddingModel::BGEBaseENV15,
                    "BGESmallENV15" => EmbeddingModel::BGESmallENV15,
                    "BGELargeENV15" => EmbeddingModel::BGELargeENV15,
                    "AllMiniLML6V2" => EmbeddingModel::AllMiniLML6V2,
                    "AllMiniLML12V2" => EmbeddingModel::AllMiniLML12V2,
                    _ => return Err(anyhow!("文本模型变体未映射: {}", entry.variant)),
                };
                Ok(Resolved::Text(m, entry.code.to_string()))
            }
            EmbeddingType::Image => {
                let m = match entry.variant {
                    "ClipVitB32" => ImageEmbeddingModel::ClipVitB32,
                    "Resnet50" => ImageEmbeddingModel::Resnet50,
                    "UnicomVitB16" => ImageEmbeddingModel::UnicomVitB16,
                    "UnicomVitB32" => ImageEmbeddingModel::UnicomVitB32,
                    "NomicEmbedVisionV15" => ImageEmbeddingModel::NomicEmbedVisionV15,
                    _ => return Err(anyhow!("图像模型变体未映射: {}", entry.variant)),
                };
                Ok(Resolved::Image(m, entry.code.to_string()))
            }
            EmbeddingType::Sparse => {
                let m = match entry.variant {
                    "SPLADEPPV1" => SparseModel::SPLADEPPV1,
                    "BGEM3" => SparseModel::BGEM3,
                    _ => return Err(anyhow!("稀疏模型变体未映射: {}", entry.variant)),
                };
                Ok(Resolved::Sparse(m, entry.code.to_string()))
            }
        };
    }
    // 目录外：用 fastembed FromStr 校验
    match t {
        EmbeddingType::Text => EmbeddingModel::from_str(input)
            .map(|m| Resolved::Text(m, input.to_string()))
            .map_err(|_| anyhow!("未知文本模型: {}", input)),
        EmbeddingType::Image => ImageEmbeddingModel::from_str(input)
            .map(|m| Resolved::Image(m, input.to_string()))
            .map_err(|_| anyhow!("未知图像模型: {}", input)),
        EmbeddingType::Sparse => SparseModel::from_str(input)
            .map(|m| Resolved::Sparse(m, input.to_string()))
            .map_err(|_| anyhow!("未知稀疏模型: {}", input)),
    }
}

/// 仅解析规范化的模型代码（不触发初始化），供 CLI 展示用。
/// 返回 None 表示输入既不在目录、也无法被 fastembed FromStr 接受。
pub(crate) fn resolve_code_for_display(t: EmbeddingType, input: &str) -> Option<String> {
    resolve(t, input).ok().map(|r| r.code().to_string())
}

// ────────────────────────────────────────────────────────────────────────────
// GPU execution providers
// ────────────────────────────────────────────────────────────────────────────

/// 按 device 配置解析 ort execution providers（GPU 加速）
/// device: "auto"（按平台自动选）| "cpu" | "coreml" | "cuda" | "directml"
pub fn resolve_execution_providers(device: &str) -> Vec<ExecutionProviderDispatch> {
    match device {
        "cpu" | "" => vec![],
        "coreml" => vec![ort::ep::CoreML::default().into()],
        "cuda" => vec![ort::ep::CUDA::default().into()],
        "directml" => vec![ort::ep::DirectML::default().into()],
        "auto" => resolve_auto_providers(),
        other => {
            tracing::warn!("未知 device '{}', 回退 CPU", other);
            vec![]
        }
    }
}

/// auto 模式：按编译目标平台自动选 GPU execution provider
fn resolve_auto_providers() -> Vec<ExecutionProviderDispatch> {
    // 每个 cfg 分支返回 (日志标签, EP 列表)；仅一个分支被编译
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
                "CUDA (Linux NVIDIA GPU, 若不可用 ort 自动回退 CPU)",
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
            ("CPU (未知平台)", vec![])
        }
    };
    tracing::info!("auto → {}", label);
    eps
}

// ────────────────────────────────────────────────────────────────────────────
// 初始化
// ────────────────────────────────────────────────────────────────────────────

/// 获取或初始化模型
///
/// 返回 `(实例池, 模型信息)`。缓存键为 `(类型, 规范化模型代码)`，
/// 因此同名变体名与模型代码共享同一池。池内含 `pool_size` 个独立实例。
///
/// **并发**：采用 double-checked locking——快路径无锁查缓存；未命中时持
/// 全局 `INIT_LOCK` 后二次检查再初始化，确保同一模型的并发首次加载串行
/// （避免 ort 并发初始化同一缓存模型时冲突失败）。运行时 embed 走快路径，
/// 不受 `INIT_LOCK` 影响；推理时从池中 round-robin 取实例，最多 N 路并发。
pub fn get_or_init_model(
    model_type: EmbeddingType,
    model_input: &str,
    cache_dir: Option<String>,
    max_length: Option<usize>,
    device: &str,
    pool_size: usize,
    show_progress: bool,
) -> Result<(Arc<ModelPool>, ModelInfo)> {
    let pool_size = pool_size.max(1); // 0 视为 1
    // 一次解析：得到类型化模型 + 规范化代码（后续不再重复解析）
    let resolved = resolve(model_type, model_input)?;
    let code = resolved.code().to_string();
    let cache_key = (model_type, code.clone());

    // 快路径：命中缓存（无锁）
    if let Some(existing) = MODEL_CACHE.get(&cache_key) {
        tracing::debug!("Get model from cache: {:?}", cache_key);
        return Ok((existing.clone(), ModelInfo::from_catalog(model_type, &code)));
    }

    // 慢路径：持初始化锁，序列化同模型的并发首次加载
    let _guard = INIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // 二次检查：持锁期间可能已被并发方写入缓存
    if let Some(existing) = MODEL_CACHE.get(&cache_key) {
        tracing::debug!("Get model from cache (post-lock): {:?}", cache_key);
        return Ok((existing.clone(), ModelInfo::from_catalog(model_type, &code)));
    }

    tracing::info!(
        "Initialization model: {:?}, device: {}, pool_size: {}",
        cache_key,
        device,
        pool_size
    );
    // 构建 N 实例池：每个实例独立加载 ONNX 会话（N× 内存），复用同一解析结果
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

    tracing::info!(
        "Model initialization successful: {:?} ({} instance(s))",
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
    TextEmbedding::try_new(options).with_context(|| format!("无法初始化文本模型: {:?}", model))
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
    ImageEmbedding::try_new(options).with_context(|| format!("无法初始化图像模型: {:?}", model))
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
        .with_context(|| format!("无法初始化稀疏模型: {:?}", model))
}

// ────────────────────────────────────────────────────────────────────────────
// ModelInfo
// ────────────────────────────────────────────────────────────────────────────

/// 模型信息
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ModelInfo {
    /// 嵌入类型
    #[schema(example = "text")]
    pub r#type: String,

    /// 模型变体名称（目录外模型回退为模型代码）
    #[schema(example = "BGELargeZHV15")]
    pub variant: String,

    /// 模型代码（Hugging Face 仓库）
    #[schema(example = "Xenova/bge-large-zh-v1.5")]
    pub code: String,

    /// 向量维度（稀疏模型为 0）
    #[schema(example = 1024)]
    pub dim: usize,
}

impl ModelInfo {
    /// 按类型与规范化代码构造 ModelInfo（从目录取维度，目录外 dim=0）
    pub fn from_catalog(model_type: EmbeddingType, code: &str) -> Self {
        let entry = catalog_for(model_type)
            .iter()
            .copied()
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

// ────────────────────────────────────────────────────────────────────────────
// 已下载模型列表
// ────────────────────────────────────────────────────────────────────────────

/// 列出本地已下载的模型（仅离线检查）
pub fn list_available_models(model_type: EmbeddingType, cache_dir: &str) -> Result<Vec<ModelInfo>> {
    let cache_path = PathBuf::from(cache_dir);

    // 如果缓存目录不存在，返回空列表
    if !cache_path.exists() {
        return Ok(vec![]);
    }

    let available: Vec<ModelInfo> = catalog_for(model_type)
        .iter()
        .copied()
        .filter(|entry| check_model_files_exist(&cache_path, entry))
        .map(|entry| ModelInfo {
            r#type: model_type.to_string(),
            variant: entry.variant.to_string(),
            code: entry.code.to_string(),
            dim: entry.dim,
        })
        .collect();

    Ok(available)
}

/// 检查模型文件是否存在
/// 验证 hf-hub 缓存结构 `models--<ns>--<repo>/snapshots/<hash>/` 下包含 onnx 模型文件
fn check_model_files_exist(cache_path: &Path, entry: &ModelEntry) -> bool {
    // "Xenova/bge-large-zh-v1.5" -> "models--Xenova--bge-large-zh-v1.5"
    let model_dir_name = format!("models--{}", entry.code.replace('/', "--"));
    let model_dir = cache_path.join(&model_dir_name);

    if !model_dir.exists() || !model_dir.is_dir() {
        return false;
    }

    let snapshots = model_dir.join("snapshots");
    if !snapshots.is_dir() {
        tracing::debug!("模型目录缺少 snapshots/: {}", model_dir.display());
        return false;
    }

    // 至少一个 snapshot 目录下存在 .onnx 文件
    let snapshots_iter = match std::fs::read_dir(&snapshots) {
        Ok(it) => it,
        Err(e) => {
            tracing::debug!("读取 snapshots 失败 {}: {}", snapshots.display(), e);
            return false;
        }
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

/// 递归（一层）检查目录下是否存在 .onnx 文件
fn dir_has_onnx(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry_res in entries.flatten() {
        let path = entry_res.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("onnx") {
            return true;
        }
        // onnx/ 子目录（部分模型把权重放在 onnx/ 下）
        if path.is_dir() && dir_has_onnx(&path) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── EmbeddingType ───────────────────────────────────────────────────────

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
        // 大小写不敏感
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
        assert!(EmbeddingType::from_str("audio").is_err());
        assert!(EmbeddingType::from_str("").is_err());
    }

    #[test]
    fn embedding_type_serde_lowercase() {
        let json = serde_json::to_string(&EmbeddingType::Sparse).unwrap();
        assert_eq!(json, "\"sparse\"");
        let t: EmbeddingType = serde_json::from_str("\"image\"").unwrap();
        assert_eq!(t, EmbeddingType::Image);
    }

    #[test]
    fn embedding_type_display() {
        assert_eq!(EmbeddingType::Text.to_string(), "text");
        assert_eq!(EmbeddingType::Image.to_string(), "image");
        assert_eq!(EmbeddingType::Sparse.to_string(), "sparse");
    }

    // ── 模型解析 ─────────────────────────────────────────────────────────────

    #[test]
    fn resolve_text_variant_and_code() {
        // 变体名 → 解析出模型 + 规范化代码
        match resolve(EmbeddingType::Text, "AllMiniLML6V2").unwrap() {
            Resolved::Text(m, code) => {
                assert_eq!(m, EmbeddingModel::AllMiniLML6V2);
                assert_eq!(code, "Qdrant/all-MiniLM-L6-v2-onnx");
            }
            other => panic!("期望 Text 变体，得到 {:?}", other.code()),
        }
        // 模型代码（命中目录；与 fastembed 实际下载仓库一致）
        match resolve(EmbeddingType::Text, "Qdrant/all-MiniLM-L6-v2-onnx").unwrap() {
            Resolved::Text(m, code) => {
                assert_eq!(m, EmbeddingModel::AllMiniLML6V2);
                assert_eq!(code, "Qdrant/all-MiniLM-L6-v2-onnx");
            }
            other => panic!("期望 Text 变体，得到 {:?}", other.code()),
        }
        match resolve(EmbeddingType::Text, "Xenova/bge-large-zh-v1.5").unwrap() {
            Resolved::Text(m, _) => assert_eq!(m, EmbeddingModel::BGELargeZHV15),
            other => panic!("期望 Text 变体，得到 {:?}", other.code()),
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
            other => panic!("期望 Image 变体，得到 {:?}", other.code()),
        }
        match resolve(EmbeddingType::Image, "Qdrant/clip-ViT-B-32-vision").unwrap() {
            Resolved::Image(m, _) => assert_eq!(m, ImageEmbeddingModel::ClipVitB32),
            other => panic!("期望 Image 变体，得到 {:?}", other.code()),
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
            other => panic!("期望 Sparse 变体，得到 {:?}", other.code()),
        }
        match resolve(EmbeddingType::Sparse, "BAAI/bge-m3").unwrap() {
            Resolved::Sparse(m, _) => assert_eq!(m, SparseModel::BGEM3),
            other => panic!("期望 Sparse 变体，得到 {:?}", other.code()),
        }
        assert!(resolve(EmbeddingType::Sparse, "nope").is_err());
    }

    // ── resolve_code ───────────────────────────────────────────────────────

    #[test]
    fn resolve_code_catalog_canonicalizes_variant() {
        // 变体名 → 规范化代码
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "AllMiniLML6V2"),
            Some("Qdrant/all-MiniLM-L6-v2-onnx".to_string())
        );
        // 代码本身 → 原样返回（命中目录）
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "Xenova/bge-large-zh-v1.5"),
            Some("Xenova/bge-large-zh-v1.5".to_string())
        );
        // 目录外、且 fastembed 不识别 → None
        assert_eq!(
            resolve_code_for_display(EmbeddingType::Text, "nonsense-xyz"),
            None
        );
    }

    // ── ModelInfo ───────────────────────────────────────────────────────────

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
        // 目录外代码：variant=code, dim=0
        let info = ModelInfo::from_catalog(EmbeddingType::Text, "some-unknown/code");
        assert_eq!(info.variant, "some-unknown/code");
        assert_eq!(info.dim, 0);
    }

    // ── resolve_execution_providers ─────────────────────────────────────────

    #[test]
    fn execution_providers_mapping() {
        assert!(resolve_execution_providers("cpu").is_empty());
        assert!(resolve_execution_providers("").is_empty());
        assert!(!resolve_execution_providers("coreml").is_empty());
        assert!(!resolve_execution_providers("cuda").is_empty());
        assert!(!resolve_execution_providers("directml").is_empty());
        // 未知 device 回退 CPU
        assert!(resolve_execution_providers("tpu").is_empty());
        // auto 的返回值取决于编译目标平台（macos/linux/windows 非空，其他空），
        // 这里仅确认调用不 panic。
        let _auto_eps = resolve_execution_providers("auto");
    }

    // ── check_model_files_exist ─────────────────────────────────────────────

    fn entry(code: &'static str) -> ModelEntry {
        ModelEntry {
            variant: "TEST",
            code,
            dim: 0,
        }
    }

    #[test]
    fn check_model_files_detects_onnx() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();

        // 构造 hf-hub 缓存结构：models--Xenova--bge-small-en-v1.5/snapshots/abc/model.onnx
        let code = "Xenova/bge-small-en-v1.5";
        let dir_name = format!("models--{}", code.replace('/', "--"));
        let snapshot = cache.join(dir_name).join("snapshots").join("abc123");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("model.onnx"), b"fake").unwrap();

        assert!(check_model_files_exist(cache, &entry(code)));
    }

    #[test]
    fn check_model_files_missing_onnx() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path();
        let code = "Xenova/bge-small-en-v1.5";
        let dir_name = format!("models--{}", code.replace('/', "--"));
        // snapshot 存在但无 onnx
        let snapshot = cache.join(dir_name).join("snapshots").join("abc");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("tokenizer.json"), b"fake").unwrap();

        assert!(!check_model_files_exist(cache, &entry(code)));
    }

    #[test]
    fn check_model_files_missing_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!check_model_files_exist(
            tmp.path(),
            &entry("Xenova/none-v1.5")
        ));
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
        // 为 AllMiniLML6V2（code: Qdrant/all-MiniLM-L6-v2-onnx）放一个 onnx
        let code = "Qdrant/all-MiniLM-L6-v2-onnx";
        let dir_name = format!("models--{}", code.replace('/', "--"));
        let snapshot = cache.join(dir_name).join("snapshots").join("h");
        std::fs::create_dir_all(&snapshot).unwrap();
        std::fs::write(snapshot.join("model.onnx"), b"x").unwrap();

        let models = list_available_models(EmbeddingType::Text, cache.to_str().unwrap()).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].variant, "AllMiniLML6V2");
        assert_eq!(models[0].dim, 384);
    }

    // ── 集成冒烟测试（需联网下载模型；默认忽略） ───────────────────────────────

    #[test]
    #[ignore = "需要联网下载 AllMiniLML6V2 模型；用 cargo test -- --ignored embed_smoke 运行"]
    fn embed_smoke_text() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().to_str().unwrap().to_string();
        let (pool, info) = get_or_init_model(
            EmbeddingType::Text,
            "AllMiniLML6V2",
            Some(cache.clone()),
            None,
            "cpu",
            1,
            true,
        )
        .expect("模型初始化失败（确认网络可用）");

        assert_eq!(info.dim, 384);
        assert_eq!(pool.len(), 1);
        let instance = pool.pick();
        let mut guard = instance.lock().unwrap();
        let out = guard
            .embed(vec!["hello world".to_string()], Some(1))
            .expect("嵌入失败");
        match out {
            EmbedOutput::Dense(vec) => {
                assert_eq!(vec.len(), 1);
                assert_eq!(vec[0].len(), 384);
            }
            EmbedOutput::Sparse(_) => panic!("text 模型应返回稠密向量"),
        }
        drop(guard);

        // 回归保护：下载后 list_available_models 必须能识别该模型。
        // 若目录 code 与 fastembed 实际下载仓库不一致，这里会失败。
        let listed = list_available_models(EmbeddingType::Text, &cache).unwrap();
        assert!(
            listed.iter().any(|m| m.variant == "AllMiniLML6V2"),
            "list_available_models 未识别已下载的 AllMiniLML6V2，目录 code 可能与 fastembed 实际仓库不一致: {:?}",
            listed
        );
    }
}
