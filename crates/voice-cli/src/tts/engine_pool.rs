//! sherpa-onnx `OfflineTts` 引擎池（镜像 STT `engine_pool` + fastembed `ModelPool` 模式）。
//!
//! 设计要点：
//! - 全局 `LazyLock<DashMap<EngineKey, Arc<EnginePool>>>` 按 model_id 缓存
//! - double-checked `INIT_LOCK`：序列化同一模型的并发首次加载（ OfflineTts::create 阻塞 IO）
//! - **不用** DashMap entry api：entry 闭包持 shard 锁期间不能再拿 `INIT_LOCK`（嵌套死锁，
//!   正是 CLAUDE.md 警告）。用 get→lock→get→insert 的 double-checked
//! - 单实例 `Arc<Mutex<OfflineTts>>` 串行。`OfflineTts: Send+Sync`（官方手动 unsafe impl），
//!   `generate_with_config(&self)` 取 `&self`（非 `&mut`），但仍用 `Mutex`：合成语义上是写，
//!   并发调同实例收益小风险大
//! - `pool_size > 1` → N 个独立实例 round-robin，允许 N 路并发（代价 N× 内存）

use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use sherpa_onnx::{
    OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig,
    OfflineTtsZipvoiceModelConfig,
};

use crate::tts::error::TtsError;
use crate::tts::model_service::{TtsModelPaths, path_to_opt_string};

/// 单个 sherpa-onnx TTS 引擎实例。
pub type EngineInstance = Arc<Mutex<OfflineTts>>;

/// TTS 引擎池（N 个独立实例，round-robin 分配）。
/// 复用 [`crate::pool::Pool`]：`Pool<OfflineTts>` 的 `pick` 返回 `EngineInstance`。
///
/// - `pool_size = 1`：单实例串行（CPU 推理通常最优）
/// - `pool_size > 1`：N 路并发（每实例独立加载一份模型，N× 内存）
pub type EnginePool = crate::pool::Pool<OfflineTts>;

/// 引擎缓存键：模型 id（对应 `{models_dir}/{model_id}/`）。
///
/// **假设**：单进程 config 固定，model_id（目录名）唯一标识一个引擎实例 —— Kokoro/ZipVoice
/// 布局不同 → model_id 天然不同；同引擎不同版本（v1_0/v1_1）model_id 也不同。故仅按 model_id
/// 索引不会串台。若未来支持“运行时热切换 config”或“同 model_id 多路径”，需扩展 key（加 backend
/// 或 paths hash）—— 当前无此需求，保持与 STT [`crate::stt::EngineKey`] 对称。
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

/// 全局引擎缓存：按 model_id 索引，每项是 N 实例池。
static TTS_CACHE: LazyLock<DashMap<EngineKey, Arc<EnginePool>>> = LazyLock::new(DashMap::new);

/// 已加载的 TTS 引擎 model_id 列表（按缓存实际状态，供 /health、/models 查询）
pub fn loaded_model_ids() -> Vec<String> {
    TTS_CACHE
        .iter()
        .map(|kv| kv.key().model_id.clone())
        .collect()
}

/// 初始化串行锁（double-checked locking 用）。
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// ZipVoice 引擎 model-level 超参（feat_scale/t_shift/target_rms/guidance_scale）。
///
/// sherpa-onnx Rust `Default`=0.0 会被 C++ Validate 拒绝（要求 >0），故必须显式设；
/// 从 `config.tts.engine.zipvoice` 填，Kokoro 时填默认值（不使用）。
#[derive(Debug, Clone, Copy)]
pub struct ZipVoiceEngineParams {
    pub feat_scale: f32,
    pub t_shift: f32,
    pub target_rms: f32,
    pub guidance_scale: f32,
}

/// TTS 引擎加载参数（model-level；per-request 参数走 `TtsOptions`）。
#[derive(Debug, Clone)]
pub struct EngineLoadParams {
    /// 模型文件路径集合（按引擎分；来自 `TtsModelService::resolve_paths`）
    pub paths: TtsModelPaths,
    /// ONNX runtime 线程数（0 = sherpa-onnx 默认）
    pub num_threads: i32,
    /// 时长缩放（model-level；Kokoro 专用，per-request 不覆盖）
    pub length_scale: f32,
    /// ONNX EP provider（v1 CPU = `None`；v2 GPU 走 `"coreml"`/`"cuda"`/`"vulkan"`）
    pub provider: Option<String>,
    /// 引擎池大小
    pub pool_size: usize,
    /// debug 模式（sherpa-onnx C 端 verbose 日志）
    pub debug: bool,
    /// Kokoro 语种（多语 v1.0 必需，如 `"mixed"`/`"zh"`/`"en"`；None 时 C 端可能 std::exit）
    pub lang: Option<String>,
    /// ZipVoice model-level 超参（Kokoro 时填默认值，不使用）
    pub zipvoice: ZipVoiceEngineParams,
}

/// 获取或初始化 TTS 引擎池（幂等；同 key 首次调用加载模型，后续命中缓存）。
///
/// 模型加载是阻塞 IO（数百 ms ~ 数秒），调用方应在 `spawn_blocking` 中调用。
pub fn get_or_init_engine(
    key: EngineKey,
    params: EngineLoadParams,
) -> Result<Arc<EnginePool>, TtsError> {
    // 快路径：命中缓存
    if let Some(p) = TTS_CACHE.get(&key) {
        return Ok(p.clone());
    }

    // 慢路径：double-checked locking
    let _g = INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(p) = TTS_CACHE.get(&key) {
        return Ok(p.clone());
    }

    // Fail Fast：加载前按引擎校验必备文件（避免在 sherpa-onnx C 端才失败，错误不可读）
    validate_paths(&params.paths)?;

    let pool_size = params.pool_size.max(1);
    let mut instances = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let tts = build_engine(&key.model_id, params.clone())?;
        tracing::debug!(
            "TTS engine pool {}/{} ready (model={})",
            i + 1,
            pool_size,
            key.model_id
        );
        instances.push(Arc::new(Mutex::new(tts)));
    }
    let pool = Arc::new(EnginePool::new(instances));
    TTS_CACHE.insert(key, pool.clone());
    Ok(pool)
}

/// 构造单个 `OfflineTts` 实例（集中所有 sherpa-onnx 调用，隔离版本 breaking）。
///
/// sherpa-onnx `OfflineTts::create` 返回 `Option`（C 端错误打 stderr 拿不到），
/// 这里映射成 `TtsError::InitFailed` 带模型 / 路径上下文。
fn build_engine(model_id: &str, params: EngineLoadParams) -> Result<OfflineTts, TtsError> {
    // 通用字段（线程/provider/debug）两引擎共用，先拼进 model_cfg；引擎专属子配置按 paths 覆盖。
    let mut model_cfg = OfflineTtsModelConfig {
        num_threads: params.num_threads,
        debug: params.debug,
        provider: params.provider,
        ..Default::default()
    };
    match params.paths {
        TtsModelPaths::Kokoro(k) => {
            model_cfg.kokoro = OfflineTtsKokoroModelConfig {
                model: path_to_opt_string(&k.model),
                voices: path_to_opt_string(&k.voices),
                tokens: path_to_opt_string(&k.tokens),
                data_dir: path_to_opt_string(&k.data_dir),
                length_scale: params.length_scale,
                dict_dir: k.dict_dir.as_ref().and_then(|p| path_to_opt_string(p)),
                // lexicon 已在 resolve_paths 拼成逗号分隔字符串（多语 Kokoro v1.0 必需，否则 C 端 exit）
                lexicon: k.lexicon.clone(),
                // lang：多语 Kokoro v1.0 必需（None 时 C 端 std::exit，整个进程挂）
                lang: params.lang.clone(),
            };
        }
        TtsModelPaths::ZipVoice(z) => {
            let zv = params.zipvoice;
            model_cfg.zipvoice = OfflineTtsZipvoiceModelConfig {
                tokens: path_to_opt_string(&z.tokens),
                encoder: path_to_opt_string(&z.encoder),
                decoder: path_to_opt_string(&z.decoder),
                vocoder: path_to_opt_string(&z.vocoder),
                data_dir: path_to_opt_string(&z.data_dir),
                lexicon: z.lexicon.clone(),
                // 4 个 float Rust Default=0.0 会被 C++ Validate 拒绝，必须显式设（>0）
                feat_scale: zv.feat_scale,
                t_shift: zv.t_shift,
                target_rms: zv.target_rms,
                guidance_scale: zv.guidance_scale,
            };
        }
    }
    let config = OfflineTtsConfig {
        model: model_cfg,
        ..Default::default()
    };

    OfflineTts::create(&config).ok_or_else(|| {
        TtsError::InitFailed(format!(
            "OfflineTts::create 返回 None（model={model_id}）。\
             常见原因：模型文件损坏 / onnxruntime 版本不匹配 / espeak-ng-data 缺失"
        ))
    })
}

/// Fail Fast：按引擎校验必备文件（缺失即返回带文件名的 ModelNotFound，避免 C 端不可读错误）。
fn validate_paths(paths: &TtsModelPaths) -> Result<(), TtsError> {
    match paths {
        TtsModelPaths::Kokoro(k) => {
            for (name, p) in [
                ("model.onnx", &k.model),
                ("voices.bin", &k.voices),
                ("tokens.txt", &k.tokens),
            ] {
                if !p.is_file() {
                    return Err(TtsError::ModelNotFound {
                        model: format!("{name} 缺失：{}", p.display()),
                    });
                }
            }
            if !k.data_dir.is_dir() {
                return Err(TtsError::ModelNotFound {
                    model: format!("espeak-ng-data/ 缺失：{}", k.data_dir.display()),
                });
            }
        }
        TtsModelPaths::ZipVoice(z) => {
            for (name, p) in [
                ("encoder.int8.onnx", &z.encoder),
                ("decoder.int8.onnx", &z.decoder),
                ("vocoder", &z.vocoder),
                ("tokens.txt", &z.tokens),
            ] {
                if !p.is_file() {
                    return Err(TtsError::ModelNotFound {
                        model: format!("{name} 缺失：{}", p.display()),
                    });
                }
            }
            if !z.data_dir.is_dir() {
                return Err(TtsError::ModelNotFound {
                    model: format!("espeak-ng-data/ 缺失：{}", z.data_dir.display()),
                });
            }
        }
    }
    Ok(())
}
