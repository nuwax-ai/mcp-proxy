//! # Voice Toolkit — 音频处理工具库（audio-only）
//!
//! 提供 ffmpeg-sidecar 驱动的音频工具：格式转换、重采样、元数据提取、Whisper 兼容格式化。
//!
//! STT/TTS 已由 `voice-cli` 直接使用 `transcribe-rs` / `sherpa-onnx`，本 crate 仅保留
//! `audio` 模块供音频预处理复用。

mod error;
pub use error::{Error, Result};

/// 音频处理模块：格式转换、重采样、元数据提取、Whisper 兼容格式。
#[cfg(feature = "audio")]
pub use rs_voice_toolkit_audio as audio;
