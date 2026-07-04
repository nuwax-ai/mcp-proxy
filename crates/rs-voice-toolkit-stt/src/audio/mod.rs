//! 音频处理工具模块
//!
//! 提供音频格式转换、重采样、预处理等功能

pub mod converter;
pub mod resampler;
pub mod utils;

pub use converter::AudioConverter;
pub use resampler::AudioResampler;
pub use utils::*;

// 重新导出独立音频模块的类型
pub use rs_voice_toolkit_audio::{AudioConfig, AudioFormat};
