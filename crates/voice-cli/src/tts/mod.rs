//! TTS 子系统：基于 sherpa-onnx 的本地 Kokoro 文本转语音。
//!
//! # 架构（镜像 STT `stt/` + fastembed `ModelPool` 模式）
//! - [`engine_pool`]：进程级引擎池（全局 `DashMap` 缓存 + double-checked `INIT_LOCK` + round-robin）
//! - [`model_service`]：Kokoro 模型目录解析 + 文件校验（v1 不自动下载，HF 阻断）
//! - [`synthesizer`]：封装 `generate_with_config`，预清洗 NUL + 错误映射
//! - [`audio_encode`]：f32 → WAV / PCM s16le
//! - [`options`]：`TtsOptions` → sherpa-onnx `GenerationConfig`（per-request）
//!
//! # 并发模型
//! 同步合成走 `spawn_blocking`（调用方负责，见 handler）；
//! 单引擎实例 `Arc<Mutex<OfflineTts>>` 串行（`OfflineTts: Send+Sync`，`generate_with_config(&self)`），
//! `pool_size > 1` 多实例并发。
//!
//! # 阶段
//! - **P3**：引擎池 + 同步合成（本模块 + `handlers/tts_sync`）
//! - **P4**：异步任务（`TtsApalisManager`）
//! - **P5**：流式 WS（callback 粒度实测，必要时切片 queue 降级）

pub mod audio_encode;
pub mod engine_pool;
pub mod error;
pub mod model_service;
pub mod options;
pub mod streaming;
pub mod synthesizer;

pub use audio_encode::{AudioFormat, encode, to_pcm_s16le as to_pcm_bytes};
pub use engine_pool::{
    EngineInstance, EngineKey, EngineLoadParams, EnginePool, get_or_init_engine,
};
pub use error::TtsError;
pub use model_service::{TtsModelPaths, TtsModelService};
pub use options::TtsOptions;
pub use streaming::{TtsStreamConfig, TtsStreamEvent, synthesize_streaming};
pub use synthesizer::{SynthesizedAudio, synthesize};
