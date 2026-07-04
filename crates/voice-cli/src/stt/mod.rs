//! STT 子系统：基于 transcribe-rs 的本地 Whisper 语音转文字。
//!
//! # 架构（参考 fastembed 已验证的 ModelPool 模式）
//! - [`engine_pool`]：进程级引擎池（全局 `DashMap` 缓存 + double-checked `INIT_LOCK` + round-robin）
//! - [`accel`]：启动时一次性配置全局 GPU 加速（macOS=metal/CoreML，Linux=cpu 或 `--features cuda`/`vulkan`）
//! - [`audio`]：ffmpeg-sidecar 转 16k/mono/s16le → `Vec<f32>` samples
//! - [`options`]：`SttTranscribeOptions` → transcribe-rs `WhisperInferenceParams`
//!
//! # 阶段
//! - **P0**：引擎池 + 批量转录跑通（本模块）
//! - **P2**：[`local_agreement`] + [`streaming_session`] 真流式（LocalAgreement 2 + WS）
//!
//! # 并发模型
//! 同步推理走 `spawn_blocking`（调用方负责，见 handler）；
//! 单引擎实例 `Arc<Mutex<WhisperEngine>>` 串行，`pool_size > 1` 多实例并发。

pub mod accel;
pub mod audio;
pub mod engine_pool;
pub mod error;
pub mod options;

pub use engine_pool::{EngineInstance, EngineKey, EnginePool, get_or_init_engine};
pub use error::SttError;
pub use options::SttTranscribeOptions;
