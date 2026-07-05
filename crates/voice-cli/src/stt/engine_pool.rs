//! Whisper 引擎池（镜像 fastembed `ModelPool` 模式）。
//!
//! 设计要点：
//! - 全局 `LazyLock<DashMap<EngineKey, Arc<EnginePool>>>` 按 model_id 缓存
//! - double-checked `INIT_LOCK`：序列化同一模型的并发首次加载（whisper.cpp context 初始化非并发安全）
//! - **不用** DashMap entry api：entry 闭包持 shard 锁期间不能再拿 `INIT_LOCK`（嵌套死锁）
//! - 单实例 `Arc<Mutex<WhisperEngine>>` 串行（`transcribe_with` 需 `&mut self`，单 context+state）
//! - `pool_size>1` → N 个独立实例 round-robin，允许 N 路并发（代价 N× 内存）

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use transcribe_rs::whisper_cpp::WhisperEngine;

use crate::stt::error::SttError;

/// 单个 Whisper 引擎实例（Mutex：transcribe_with 需 &mut self）。
pub type EngineInstance = Arc<Mutex<WhisperEngine>>;

/// Whisper 引擎池（N 个独立实例，round-robin 分配）。
/// 复用 [`crate::pool::Pool`]：`Pool<WhisperEngine>` 的 `pick` 返回 `EngineInstance`。
///
/// - `pool_size = 1`：单实例串行（CPU 推理通常最优，避免线程超订阅）
/// - `pool_size > 1`：N 路并发（每实例独立 context+state，N× 内存）
pub type EnginePool = crate::pool::Pool<WhisperEngine>;

/// 引擎缓存键：模型 id（对应 `ggml-{model_id}.bin`）。
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
static ENGINE_CACHE: LazyLock<DashMap<EngineKey, Arc<EnginePool>>> = LazyLock::new(DashMap::new);

/// 初始化串行锁（double-checked locking 用）。
static INIT_LOCK: Mutex<()> = Mutex::new(());

/// 获取或初始化引擎池（幂等；同 key 首次调用加载模型，后续命中缓存）。
///
/// - GPU 后端由全局 `accel::init_global_accel` 决定（需在启动早期调用一次）
/// - `flash_attn` 走 `WhisperLoadParams` 默认（true）
/// - `pool_size` 被 clamp 到 `>= 1`
pub fn get_or_init_engine(
    key: EngineKey,
    model_path: PathBuf,
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
                "加载 Whisper 模型 {}（第 {} 实例）失败: {e}",
                model_path.display(),
                i + 1
            ))
        })?;
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
