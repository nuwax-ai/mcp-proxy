//! sherpa-onnx `OfflineRecognizer` 引擎池（FireRedASR2-AED / Fun-ASR-Nano / Qwen3-ASR）。
//!
//! 镜像 TTS [`crate::tts::engine_pool`]（`OfflineTts` 池）+ 复用 [`crate::pool::Pool`]。
//! 与 transcribe-rs 的 dyn 池（whisper/sensevoice）**独立**：sherpa-onnx `OfflineRecognizer`
//! 不实现 transcribe-rs `SpeechModel` trait，且返回 `OfflineRecognizerResult`（非
//! `TranscriptionResult`），故单列一池，避免动已验证的 whisper/sensevoice 路径。
//!
//! # 设计要点（与 TTS engine_pool 同构）
//! - 全局 `LazyLock<DashMap<EngineKey, Arc<SherpaEnginePool>>>` 按 model_id 缓存
//! - double-checked `INIT_LOCK`：序列化同一模型的并发首次加载（`OfflineRecognizer::create` 阻塞 IO）
//! - **不用** DashMap entry api：entry 闭包持 shard 锁期间不能再拿 `INIT_LOCK`（嵌套死锁，
//!   CLAUDE.md 警告）。用 get→lock→get→insert 的 double-checked
//! - 单实例 `Arc<Mutex<OfflineRecognizer>>` 串行（`OfflineRecognizer: Send+Sync` 官方手动 unsafe impl，
//!   `create_stream`/`decode` 取 `&self`，但仍用 Mutex：解码语义上是写，并发调同实例收益小风险大）
//! - **per-instance `provider`**（与 transcribe-rs 全局 `init_global_accel` 独立）：Mac `"coreml"` 走
//!   CoreML GPU/ANE；sherpa-onnx 预编译 macOS lib 含 CoreMLExecutionProvider（与 ort 不同）
//!
//! # ⚠️ Fun-ASR-Nano 默认值陷阱（sherpa-onnx issue #3066 根因）
//! Rust `OfflineFunASRNanoModelConfig::default()` 是错的（`max_new_tokens:0`/`temperature:1.0`/
//! 无 prompt → 乱码重复）。[`build_recognizer`] 显式覆盖 6 个标量/prompt 为 C++ 工作默认。

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use sherpa_onnx::{
    OfflineFireRedAsrModelConfig, OfflineFunASRNanoModelConfig, OfflineModelConfig,
    OfflineQwen3ASRModelConfig, OfflineRecognizer, OfflineRecognizerConfig,
    OfflineRecognizerResult,
};

use crate::pool::Pool;
use crate::stt::engine_pool::EngineKey;
use crate::stt::error::SttError;
use crate::stt::sherpa_model_paths::{SherpaAsrKind, SherpaAsrPaths, resolve_paths};

/// 单个 sherpa-onnx ASR 引擎实例。
pub type SherpaEngineInstance = Arc<Mutex<OfflineRecognizer>>;

/// sherpa-onnx ASR 引擎池（N 个独立实例，round-robin 分配）。复用 [`Pool`]。
pub type SherpaEnginePool = Pool<OfflineRecognizer>;

/// 全局 sherpa-onnx ASR 引擎缓存：按 model_id 索引，每项是 N 实例池。
static SHERPA_ASR_CACHE: LazyLock<DashMap<EngineKey, Arc<SherpaEnginePool>>> =
    LazyLock::new(DashMap::new);

/// 初始化串行锁（double-checked locking 用）。
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// 清空 sherpa-onnx ASR 引擎缓存，释放模型内存。仅供测试/基准隔离用（生产靠缓存命中）。
/// 调用方须先 drop 掉持有的 `Arc<SherpaEnginePool>` clone，clear 才真正释放模型。
pub fn clear_cache() {
    SHERPA_ASR_CACHE.clear();
}

/// sherpa-onnx ASR 引擎加载参数（model-level；per-request 语言/热词已在 kind 内）。
#[derive(Debug, Clone)]
pub struct SherpaAsrLoadParams {
    /// 模型目录（sherpa-onnx release 解压后的目录，含 model 文件 + tokens/tokenizer）
    pub model_dir: PathBuf,
    /// 引擎种类 + per-model 生成参数（hotwords / max_new_tokens）
    pub kind: SherpaAsrKind,
    /// ONNX EP provider（`None`=CPU；Mac `"coreml"`=GPU/ANE；Linux `"cuda"`=NVIDIA）
    pub provider: Option<String>,
    /// ONNX runtime 线程数（0 = sherpa-onnx 默认）
    pub num_threads: i32,
    /// 引擎池大小
    pub pool_size: usize,
    /// debug 模式（sherpa-onnx C 端 verbose 日志）
    pub debug: bool,
}

/// 获取或初始化 sherpa-onnx ASR 引擎池（幂等；同 key 首次调用加载模型，后续命中缓存）。
///
/// 模型加载是阻塞 IO（数百 ms ~ 数秒），调用方应在 `spawn_blocking` 中调用。
pub fn get_or_init_engine(
    key: EngineKey,
    params: SherpaAsrLoadParams,
) -> Result<Arc<SherpaEnginePool>, SttError> {
    // 快路径：命中缓存
    if let Some(p) = SHERPA_ASR_CACHE.get(&key) {
        return Ok(p.clone());
    }

    // 慢路径：double-checked locking
    let _g = INIT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(p) = SHERPA_ASR_CACHE.get(&key) {
        return Ok(p.clone());
    }

    // Fail Fast：加载前解析 + 校验必备文件（避免传到 sherpa-onnx C 端才报不可读错误）
    let paths = resolve_paths(&params.kind, &params.model_dir)?;

    let pool_size = params.pool_size.max(1);
    let mut instances = Vec::with_capacity(pool_size);
    for i in 0..pool_size {
        let rec = build_recognizer(&paths, &params.kind, &params.model_dir, &params)?;
        tracing::debug!(
            "sherpa-onnx ASR engine pool {}/{} ready (model={})",
            i + 1,
            pool_size,
            key.model_id
        );
        instances.push(Arc::new(Mutex::new(rec)));
    }
    let pool = Arc::new(SherpaEnginePool::new(instances));
    SHERPA_ASR_CACHE.insert(key, pool.clone());
    Ok(pool)
}

/// 构造单个 `OfflineRecognizer` 实例（集中所有 sherpa-onnx 调用，按 kind 填对应子结构）。
///
/// sherpa-onnx `OfflineRecognizer::create` 返回 `Option`（C 端错误打 stderr 拿不到），
/// 这里映射成 `SttError::InitFailed` 带模型目录上下文。
fn build_recognizer(
    paths: &SherpaAsrPaths,
    kind: &SherpaAsrKind,
    model_dir: &Path,
    params: &SherpaAsrLoadParams,
) -> Result<OfflineRecognizer, SttError> {
    let mut model_config = OfflineModelConfig {
        num_threads: params.num_threads,
        debug: params.debug,
        // per-instance provider（Mac "coreml" / Linux "cuda"）；与 transcribe-rs 全局 accel 独立
        provider: params.provider.clone(),
        ..Default::default()
    };

    match paths {
        SherpaAsrPaths::FireRedAsr2 {
            encoder,
            decoder,
            tokens,
        } => {
            model_config.fire_red_asr = OfflineFireRedAsrModelConfig {
                encoder: Some(p2s(encoder)),
                decoder: Some(p2s(decoder)),
            };
            model_config.tokens = Some(p2s(tokens));
        }
        SherpaAsrPaths::FunAsrNano {
            encoder_adaptor,
            llm,
            embedding,
            tokenizer_dir,
        } => {
            let hotwords = match kind {
                SherpaAsrKind::FunAsrNano { hotwords } => hotwords.clone(),
                _ => None,
            };
            // ⚠️ 必须显式覆盖：Rust Default 是错的（max_new_tokens:0 / temperature:1.0 / 无 prompt
            // → 乱码重复，sherpa-onnx issue #3066）。值为 C++ 工作默认。
            model_config.funasr_nano = OfflineFunASRNanoModelConfig {
                encoder_adaptor: Some(p2s(encoder_adaptor)),
                llm: Some(p2s(llm)),
                embedding: Some(p2s(embedding)),
                tokenizer: Some(p2s(tokenizer_dir)),
                system_prompt: Some("You are a helpful assistant.".to_string()),
                user_prompt: Some("语音转写：".to_string()),
                max_new_tokens: 512,
                temperature: 1e-6,
                top_p: 0.8,
                seed: 42,
                language: None,
                itn: 1,
                hotwords,
            };
        }
        SherpaAsrPaths::Qwen3Asr {
            conv_frontend,
            encoder,
            decoder,
            tokenizer_dir,
        } => {
            let (hotwords, max_new_tokens) = match kind {
                SherpaAsrKind::Qwen3Asr {
                    hotwords,
                    max_new_tokens,
                } => (hotwords.clone(), *max_new_tokens),
                _ => (None, 128),
            };
            // Qwen3 Rust Default 是 sane 的（max_total_len:512 / temperature:1e-6 / top_p:0.8 /
            // seed:42）；仅透传 max_new_tokens（config 默认 1024）与 hotwords，其余走 Default。
            model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
                conv_frontend: Some(p2s(conv_frontend)),
                encoder: Some(p2s(encoder)),
                decoder: Some(p2s(decoder)),
                tokenizer: Some(p2s(tokenizer_dir)),
                max_new_tokens,
                hotwords,
                ..Default::default()
            };
        }
    }

    let config = OfflineRecognizerConfig {
        model_config,
        ..Default::default()
    };

    OfflineRecognizer::create(&config).ok_or_else(|| {
        SttError::InitFailed(format!(
            "OfflineRecognizer::create 返回 None（sherpa ASR，model_dir={}，provider={:?}）。\
             常见原因：模型文件损坏 / onnxruntime 版本不匹配 / provider 当前构建不支持",
            model_dir.display(),
            params.provider
        ))
    })
}

/// 单段转录：create_stream → accept_waveform(16k) → decode → get_result。
///
/// `samples` 须为 16k/mono/f32（复用 [`crate::stt::audio::to_whisper_samples`]，引擎无关）。
pub fn recognize(
    rec: &OfflineRecognizer,
    samples: &[f32],
) -> Result<OfflineRecognizerResult, SttError> {
    let stream = rec.create_stream();
    stream.accept_waveform(16_000, samples);
    rec.decode(&stream);
    stream.get_result().ok_or_else(|| {
        SttError::InferFailed(
            "sherpa-onnx OfflineStream::get_result 返回 None（解码未产出结果）".into(),
        )
    })
}

/// `Path` → `String`（路径已由 [`resolve_paths`] 校验非空，直接转）。
fn p2s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}
