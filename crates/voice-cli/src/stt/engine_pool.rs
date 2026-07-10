//! STT 引擎池（批量 = dyn 池；流式 = whisper 具象池）。
//!
//! 设计要点：
//! - **批量池** `ENGINE_CACHE`：`Pool<Box<dyn SpeechModel + Send + 'static>>`，whisper / sensevoice
//!   统一为 trait object 共池（transcribe-rs 两引擎同实现 [`transcribe_rs::SpeechModel`]）。
//!   调用方走 trait 方法 `transcribe(&samples, &TranscribeOptions)`（语言等共享参数）。
//! - **流式池** `WHISPER_CACHE`：`Pool<WhisperEngine>` 具象，保留 whisper 专属 `transcribe_with`
//!   （`WhisperInferenceParams`，含 initial_prompt 等），供 LA2 token 级双解码用。
//!   SenseVoice 非流式，不进此池。
//! - 全局 `LazyLock<DashMap<EngineKey, Arc<EnginePool>>>` 按 model_id 缓存
//! - double-checked `INIT_LOCK`：序列化同一模型的并发首次加载（whisper.cpp context 初始化非并发安全）
//! - **不用** DashMap entry api：entry 闭包持 shard 锁期间不能再拿 `INIT_LOCK`（嵌套死锁）
//! - 单实例 `Arc<Mutex<T>>` 串行（`transcribe`/`transcribe_with` 需 `&mut self`）
//! - `pool_size>1` → N 个独立实例 round-robin，允许 N 路并发（代价 N× 内存）

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use transcribe_rs::SpeechModel;
use transcribe_rs::whisper_cpp::WhisperEngine;

use crate::stt::error::SttError;

/// 引擎加载规格：决定 [`get_or_init_engine`] 如何构造实例。
///
/// 两引擎 `load` 签名差异大（whisper=单文件路径，sensevoice=目录+量化），用枚举比 struct 清爽。
#[derive(Debug, Clone)]
pub enum SttEngineSpec {
    /// Whisper（GGML 单文件 `ggml-*.bin`）
    Whisper { model_path: PathBuf },
    /// SenseVoice（ONNX 目录：`model.{quant}.onnx` + `tokens.txt`）
    #[cfg(feature = "sensevoice")]
    SenseVoice {
        model_dir: PathBuf,
        quantization: transcribe_rs::onnx::Quantization,
    },
}

/// 转录分派产物：统一 transcribe-rs 与 sherpa-onnx 两类引擎的 `spawn_blocking` 入口。
///
/// - [`SttInvocation::TranscribeRs`]：走 dyn 池（whisper / sensevoice，`Box<dyn SpeechModel>`）
/// - [`SttInvocation::Sherpa`]：走 sherpa-onnx 池（FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR，
///   `OfflineRecognizer`），与 transcribe-rs 池独立
///
/// `spawn_blocking` 闭包 `match invocation` 分两条路径，共享 `to_whisper_samples`，
/// 统一返回 `TranscriptionResponse`（各自走 `map_transcription_result` / `map_sherpa_recognition_result`）。
#[derive(Debug, Clone)]
pub enum SttInvocation {
    /// transcribe-rs 引擎（whisper / sensevoice）
    TranscribeRs {
        model_id: String,
        spec: SttEngineSpec,
    },
    /// sherpa-onnx 引擎（FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR）
    Sherpa {
        model_id: String,
        load_params: crate::stt::sherpa_engine_pool::SherpaAsrLoadParams,
    },
}

// ===========================================================================
// 批量：dyn 池（whisper / sensevoice 统一为 Box<dyn SpeechModel>）
// ===========================================================================

/// 批量引擎实例（trait object；`transcribe` 需 `&mut self`）。
pub type EngineInstance = Arc<Mutex<Box<dyn SpeechModel + Send + 'static>>>;

/// 批量引擎池（N 个独立实例，round-robin 分配）。复用 [`crate::pool::Pool`]。
pub type EnginePool = crate::pool::Pool<Box<dyn SpeechModel + Send + 'static>>;

/// 引擎缓存键：模型 id（whisper = `ggml-{id}.bin` 的 id；sensevoice = 模型目录名）。
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct EngineKey {
    pub model_id: String,
}

impl EngineKey {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
        }
    }
}

/// 全局批量引擎缓存：按 model_id 索引，每项是 N 实例 dyn 池。
static ENGINE_CACHE: LazyLock<DashMap<EngineKey, Arc<EnginePool>>> = LazyLock::new(DashMap::new);

/// 批量池初始化串行锁（double-checked locking 用）。
static INIT_LOCK: Mutex<()> = Mutex::new(());

// ===========================================================================
// 流式：whisper 具象池（保留 transcribe_with / LA2）
// ===========================================================================

/// 流式 Whisper 实例（具象；`transcribe_with` 需 `&mut self`）。
pub type WhisperInstance = Arc<Mutex<WhisperEngine>>;

/// 流式 Whisper 池。SenseVoice 非流式，不进此池。
pub type WhisperPool = crate::pool::Pool<WhisperEngine>;

/// 全局流式 Whisper 缓存（独立于批量 dyn 池：流式需具象 `transcribe_with`）。
static WHISPER_CACHE: LazyLock<DashMap<EngineKey, Arc<WhisperPool>>> = LazyLock::new(DashMap::new);

/// 流式池初始化串行锁。
static WHISPER_INIT_LOCK: Mutex<()> = Mutex::new(());

/// 已加载的 STT 引擎 model_id 列表（合并批量 dyn 池 + 流式 whisper 池，供 /health、/models 查询）。
pub fn loaded_model_ids() -> Vec<String> {
    let mut ids: Vec<String> = ENGINE_CACHE
        .iter()
        .map(|kv| kv.key().model_id.clone())
        .collect();
    ids.extend(WHISPER_CACHE.iter().map(|kv| kv.key().model_id.clone()));
    ids.sort();
    ids.dedup();
    ids
}

/// 清空全部 STT 引擎缓存（批量 dyn 池 + 流式 whisper 池），释放模型内存。
///
/// 仅供测试/基准隔离用（一次只驻留一个引擎，避免多模型占满内存）；**生产路径靠缓存命中，不调用**。
/// 调用方须先 drop 掉持有的 `Arc<Pool>` clone，使缓存成为最后引用，clear 才真正释放模型。
pub fn clear_cache() {
    ENGINE_CACHE.clear();
    WHISPER_CACHE.clear();
}

/// 获取或初始化**批量**引擎池（dyn；幂等；同 key 首次调用加载模型，后续命中缓存）。
///
/// - `spec` 决定引擎类型 + 模型位置（whisper 文件 / sensevoice 目录）
/// - GPU 后端由全局 `accel::init_global_accel` 决定（whisper + ORT 两套，启动早期各调一次）
/// - `pool_size` 被 clamp 到 `>= 1`
pub fn get_or_init_engine(
    key: EngineKey,
    spec: SttEngineSpec,
    pool_size: usize,
) -> Result<Arc<EnginePool>, SttError> {
    // 快路径：命中缓存
    if let Some(p) = ENGINE_CACHE.get(&key) {
        return Ok(p.clone());
    }

    // 慢路径：double-checked locking
    let _g = INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(p) = ENGINE_CACHE.get(&key) {
        return Ok(p.clone());
    }

    let pool_size = pool_size.max(1);
    let mut instances = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let engine: Box<dyn SpeechModel + Send + 'static> = match &spec {
            SttEngineSpec::Whisper { model_path } => {
                if !model_path.exists() {
                    return Err(SttError::ModelNotFound {
                        model: model_path.display().to_string(),
                    });
                }
                let e = WhisperEngine::load(model_path).map_err(|e| {
                    SttError::InitFailed(format!(
                        "加载 Whisper 模型 {}（第 {} 实例）失败: {e}",
                        model_path.display(),
                        i + 1
                    ))
                })?;
                Box::new(e)
            }
            #[cfg(feature = "sensevoice")]
            SttEngineSpec::SenseVoice {
                model_dir,
                quantization,
            } => crate::stt::sensevoice::load(model_dir, quantization).map_err(|e| {
                SttError::InitFailed(format!(
                    "加载 SenseVoice 模型 {}（第 {} 实例）失败: {e}",
                    model_dir.display(),
                    i + 1
                ))
            })?,
        };
        tracing::debug!(
            "STT engine pool {}/{} ready (model={})",
            i + 1,
            pool_size,
            key.model_id
        );
        instances.push(Arc::new(Mutex::new(engine)));
    }
    let pool = Arc::new(EnginePool::new(instances));
    ENGINE_CACHE.insert(key, pool.clone());
    Ok(pool)
}

/// 获取或初始化**流式** Whisper 池（具象；幂等）。
///
/// 流式独立于批量 dyn 池：LA2 双解码需 whisper 专属 `transcribe_with(WhisperInferenceParams)`
/// （含 initial_prompt 等），trait object 上无法调用。SenseVoice 非流式，不进此池。
pub fn get_or_init_whisper(
    key: EngineKey,
    model_path: PathBuf,
    pool_size: usize,
) -> Result<Arc<WhisperPool>, SttError> {
    if let Some(p) = WHISPER_CACHE.get(&key) {
        return Ok(p.clone());
    }

    let _g = WHISPER_INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(p) = WHISPER_CACHE.get(&key) {
        return Ok(p.clone());
    }

    if !model_path.exists() {
        return Err(SttError::ModelNotFound {
            model: model_path.display().to_string(),
        });
    }

    let pool_size = pool_size.max(1);
    let mut instances = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let engine = WhisperEngine::load(&model_path).map_err(|e| {
            SttError::InitFailed(format!(
                "加载（流式）Whisper 模型 {}（第 {} 实例）失败: {e}",
                model_path.display(),
                i + 1
            ))
        })?;
        tracing::debug!(
            "STT whisper(stream) pool {}/{} ready (model={})",
            i + 1,
            pool_size,
            key.model_id
        );
        instances.push(Arc::new(Mutex::new(engine)));
    }
    let pool = Arc::new(WhisperPool::new(instances));
    WHISPER_CACHE.insert(key, pool.clone());
    Ok(pool)
}
