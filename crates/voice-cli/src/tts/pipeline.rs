//! TTS 合成管线辅助：统一三处调用点（sync handler / async worker / streaming）
//! 共有的「resolve_paths + build load_params + init pool + pick」流程。
//!
//! 设计：
//! - [`acquire_instance`]：返回 `Arc<Mutex<OfflineTts>>`（池化实例）。锁留给调用方——
//!   `MutexGuard` 生命周期绑局部 `Arc`，不能跨函数返回（Rust「返回锁」标准痛点）。
//! - [`synth_to_bytes`]：整段同步合成 → bytes，封装 acquire + lock + synthesize + encode。
//!   sync handler 和 async worker共用。
//!
//! streaming 路径用 [`acquire_instance`] 取实例后自行调 `generate_with_config(callback)，
//! 因为 callback 增量推送不能走 `synthesize` 的整段封装。

use crate::models::config::TtsEngineConfig;
use crate::tts::{
    AudioFormat, EngineInstance, EngineKey, EngineLoadParams, TtsError, TtsModelService,
    TtsOptions, encode, get_or_init_engine, synthesize,
};

/// 解析模型路径 + 构造 load_params + 初始化/取池化实例 + round-robin pick。
///
/// `length_scale` 为 model-level 参数（仅引擎首次加载生效，池化后共享）。
/// 调用方拿到 `EngineInstance` 后自行 `lock()` 使用。
pub fn acquire_instance(
    model_svc: &TtsModelService,
    model_id: &str,
    engine: &TtsEngineConfig,
    length_scale: f32,
) -> Result<EngineInstance, TtsError> {
    let paths = model_svc.resolve_paths(model_id)?;
    let load_params = EngineLoadParams {
        paths,
        num_threads: engine.num_threads,
        length_scale,
        provider: engine.provider.clone(),
        pool_size: engine.pool_size,
        debug: engine.debug,
        lang: engine.default_language.clone(),
    };
    let pool = get_or_init_engine(EngineKey::new(model_id), load_params)?;
    Ok(pool.pick())
}

/// 同步合成整段 → 音频 bytes（统一 sync handler + async worker 路径）。
///
/// 返回 `(bytes, sample_rate, n_samples)`。在 `spawn_blocking` 中调用。
pub fn synth_to_bytes(
    model_svc: &TtsModelService,
    model_id: &str,
    engine: &TtsEngineConfig,
    text: &str,
    opts: &TtsOptions,
    length_scale: f32,
    fmt: AudioFormat,
) -> Result<(Vec<u8>, i32, usize), TtsError> {
    let inst = acquire_instance(model_svc, model_id, engine, length_scale)?;
    let guard = inst.lock().unwrap_or_else(|p| p.into_inner());
    let audio = synthesize(&guard, text, opts)?;
    let n_samples = audio.samples.len();
    let sample_rate = audio.sample_rate;
    let bytes = encode(&audio.samples, sample_rate, fmt)?;
    Ok((bytes, sample_rate, n_samples))
}
