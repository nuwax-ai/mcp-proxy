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
pub mod local_agreement;
pub mod options;
pub mod script_convert;
#[cfg(feature = "sensevoice")]
pub mod sensevoice;
pub mod sherpa_engine_pool;
pub mod sherpa_model_paths;
pub mod streaming_session;

pub use engine_pool::{
    EngineInstance, EngineKey, EnginePool, SttEngineSpec, SttInvocation, WhisperInstance,
    WhisperPool, get_or_init_engine, get_or_init_whisper,
};
pub use error::SttError;
pub use local_agreement::{CompareGranularity, LaConfig, LaDecision, LocalAgreement};
pub use options::SttTranscribeOptions;
pub use script_convert::{convert_if_needed, to_simplified};
pub use sherpa_engine_pool::{
    SherpaAsrLoadParams, SherpaEngineInstance, SherpaEnginePool,
    get_or_init_engine as sherpa_get_or_init_engine, recognize as sherpa_recognize,
};
pub use sherpa_model_paths::{SherpaAsrKind, SherpaAsrPaths};
pub use streaming_session::{
    Decoder, SessionConfig, SessionError, StreamEvent, StreamingSession, WhisperDecoder,
};

use crate::models::config::OutputScript;
use crate::models::request::{Segment, TranscriptionResponse};
use sherpa_onnx::OfflineRecognizerResult;

/// 清空全部 STT 引擎缓存（transcribe-rs dyn 池 + 流式 whisper 池 + sherpa-onnx 池），释放模型内存。
///
/// 仅供测试/基准隔离用（如 benches/stt_engines.rs 一次只驻留一个引擎，避免多模型占满内存）；
/// **生产路径靠缓存命中，不调用**。调用方须先 drop 掉持有的池 Arc clone。
pub fn clear_all_caches() {
    engine_pool::clear_cache();
    sherpa_engine_pool::clear_cache();
}

/// 把 transcribe-rs [`transcribe_rs::TranscriptionResult`] 映射为 HTTP `TranscriptionResponse`，
/// 并按 `output_script` 对 `text` + `segments[].text` 做繁→简转换。
///
/// `TranscriptionResult` 是 transcribe-rs **顶层共享类型**（whisper 与 sensevoice 同型返回），
/// 故本函数引擎无关，两引擎批量转录结果统一走这里。统一同步（`/transcribe`）与异步
/// （`/api/v1/tasks/transcribe`、`transcribeFromUrl`）三处映射，消除重复代码；异步存库即简体
/// （取结果端点无需再转）。返回 response 的 `language`/`duration`/`processing_time`/`metadata`
/// 留空，由调用方按上下文补（如同步 handler 补 metadata）。
pub fn map_transcription_result(
    result: transcribe_rs::TranscriptionResult,
    script: OutputScript,
) -> TranscriptionResponse {
    TranscriptionResponse {
        text: convert_if_needed(&result.text, script),
        segments: result
            .segments
            .unwrap_or_default()
            .into_iter()
            .map(|s| Segment {
                start: s.start,
                end: s.end,
                text: convert_if_needed(&s.text, script),
                confidence: 0.0, // transcribe-rs 0.3.11 TranscriptionSegment 无 confidence
            })
            .collect(),
        language: None,
        duration: None,
        processing_time: 0.0,
        metadata: None,
    }
}

/// 把 sherpa-onnx [`OfflineRecognizerResult`] 映射为 HTTP `TranscriptionResponse`，
/// 并按 `output_script` 对 `text` 做繁→简转换。
///
/// 与 [`map_transcription_result`] 对称（统一同步/异步三处映射），但**源类型不同**：
/// sherpa-onnx 返回 `OfflineRecognizerResult`（非 transcribe-rs `TranscriptionResult`）。
/// AED/LLM-decoder 模型（FireRedASR2-AED / Fun-ASR-Nano / Qwen3-ASR）不返回 segment 级
/// 时间戳，故 `segments` 为空。`language`/`duration`/`processing_time`/`metadata` 留空，
/// 由调用方按上下文补（如同步 handler 补 metadata）。
pub fn map_sherpa_recognition_result(
    result: OfflineRecognizerResult,
    script: OutputScript,
) -> TranscriptionResponse {
    TranscriptionResponse {
        text: convert_if_needed(&result.text, script),
        segments: Vec::new(),
        language: None,
        duration: None,
        processing_time: 0.0,
        metadata: None,
    }
}
