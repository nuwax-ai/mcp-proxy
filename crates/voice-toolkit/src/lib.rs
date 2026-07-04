//! # Voice Toolkit - Rust 语音处理工具库
//!
//! `voice-toolkit` 是一个功能强大的 Rust 语音处理工具库，提供统一的接口来处理语音转文本(STT)、
//! 文本转语音(TTS)和音频处理任务。该库基于 Whisper 模型提供高质量的语音识别功能，
//! 支持多种音频格式转换和实时语音处理。
//!
//! ## 主要特性
//!
//! - **语音转文本 (STT)**: 基于 OpenAI Whisper 模型的高质量语音识别
//! - **文本转语音 (TTS)**: 基于 Index-TTS 的语音合成功能
//! - **音频处理**: 支持多种音频格式的转换和处理
//! - **实时流式处理**: 支持实时音频流的转录
//! - **语音活动检测 (VAD)**: 智能检测语音片段，提高处理效率
//! - **跨平台支持**: 支持 Windows、macOS 和 Linux
//! - **GPU 加速**: 可选的 CUDA、Vulkan 和 Metal 加速支持
//!
//! ## 快速开始
//!
//! ### 基本依赖
//!
//! 在 `Cargo.toml` 中添加依赖：
//!
//! ```toml
//! [dependencies]
//! voice-toolkit = { version = "0.15.0", features = ["stt", "tts", "audio"] }
//! ```
//!
//! ### 语音转文本示例
//!
//! ```ignore
//! use voice_toolkit::transcribe_file_unified;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let model_path = "models/ggml-base.bin";
//!     let audio_path = "audio/hello.wav";
//!     
//!     let result = transcribe_file_unified(model_path, audio_path).await?;
//!     println!("转录结果: {}", result.text);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 音频格式转换示例
//!
//! ```ignore
//! use voice_toolkit::audio;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let input_path = "audio/input.mp3";
//!     let output_path = "audio/output.wav";
//!     
//!     // 将 MP3 转换为 Whisper 兼容的 WAV 格式
//!     audio::convert_to_whisper_format(input_path, output_path).await?;
//!     println!("转换完成: {}", output_path);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 文本转语音示例
//!
//! ```ignore
//! // 需要 `tts` feature：voice-toolkit 默认只开 stt+audio，故本示例标记为 ignore
//! use voice_toolkit::tts;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let text = "你好，世界！欢迎使用语音工具库。";
//!     let output_path = "output/hello.wav";
//!     
//!     // 使用 Index-TTS 生成语音
//!     tts::synthesize_text(text, output_path, None).await?;
//!     println!("语音合成完成: {}", output_path);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## 功能特性
//!
//! ### STT 功能
//! - 支持多种音频格式：WAV、MP3、FLAC、M4A、OGG 等
//! - 自动音频格式转换和预处理
//! - 支持实时流式转录
//! - 集成语音活动检测 (VAD)
//! - 性能监控和基准测试
//!
//! ### TTS 功能
//! - 基于 Index-TTS 的高质量语音合成
//! - 支持多种输出格式
//! - 可扩展的引擎架构
//!
//! ### 音频处理功能
//! - 音频格式转换
//! - 音频重采样
//! - 元数据提取
//! - Whisper 兼容格式转换
//!
//! ## 特性标志
//!
//! 该库使用特性标志来控制功能模块的启用：
//!
//! - `stt`: 启用语音转文本功能（默认启用）
//! - `tts`: 启用文本转语音功能
//! - `audio`: 启用音频处理功能（默认启用）
//! - `streaming`: 启用实时流式处理（需要 `stt`）
//! - `cuda`: 启用 CUDA GPU 加速（需要 `stt`）
//! - `vulkan`: 启用 Vulkan GPU 加速（需要 `stt`）
//! - `metal`: 启用 Metal GPU 加速（需要 `stt`）
//!
//! ## 系统要求
//!
//! - **Rust**: 1.70 或更高版本
//! - **FFmpeg**: 用于音频处理
//!   - macOS: `brew install ffmpeg`
//!   - Ubuntu: `sudo apt-get install ffmpeg`
//!   - Windows: 使用 vcpkg 安装
//! - **Whisper 模型**: 需要下载 Whisper 模型文件（.bin 格式）
//!
//! ## 错误处理
//!
//! 该库使用统一的错误处理机制，所有函数都返回 `Result<T, Error>` 类型，
//! 其中 `Error` 是一个枚举类型，包含了所有可能的错误情况。
//!
//! ## 性能考虑
//!
//! - 首次加载模型时会有一定的延迟
//! - 建议在长期运行的应用中复用模型实例
//! - 使用 GPU 加速可以显著提高处理速度
//! - 对于实时应用，建议使用流式处理功能
//!
//! ## 许可证
//!
//! 本项目采用 MIT 或 Apache 2.0 许可证。详情请参阅 [LICENSE](LICENSE) 文件。
//!
//! ## 贡献
//!
//! 欢迎提交 Issue 和 Pull Request！请参阅 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。
//!
//! ## 更新日志
//!
//! 请参阅 [CHANGELOG.md](CHANGELOG.md) 了解版本更新详情。

// 导入错误处理模块
mod error;
pub use error::{Error, Result};

// 重新导出各个模块
/// 语音转文本 (STT) 模块
///
/// 该模块提供基于 OpenAI Whisper 模型的语音识别功能，包括：
/// - 文件转录：处理音频文件并转换为文本
/// - 流式转录：实时处理音频流
/// - 语音活动检测 (VAD)：智能检测语音片段
/// - 性能监控：提供转录性能指标
#[cfg(feature = "stt")]
pub use rs_voice_toolkit_stt as stt;

/// 音频处理模块
///
/// 该模块提供音频文件的格式转换、重采样和元数据提取功能，包括：
/// - 多格式支持：WAV、MP3、FLAC、M4A、OGG 等
/// - 音频重采样：支持多种采样率转换
/// - 元数据提取：获取音频文件的详细信息
/// - Whisper 兼容格式：自动转换为 Whisper 识别所需的格式
#[cfg(feature = "audio")]
pub use rs_voice_toolkit_audio as audio;

/// 文本转语音 (TTS) 模块
///
/// 该模块提供文本到语音的转换功能，包括：
/// - Index-TTS 引擎：基于 Index-TTS 的高质量语音合成
/// - 多种输出格式：支持 WAV、MP3 等格式
/// - 可扩展架构：支持多种 TTS 引擎
/// - 灵活配置：支持语音速度、音调等参数调整
#[cfg(feature = "tts")]
pub use rs_voice_toolkit_tts as tts;

// 重新导出常用的类型和函数
/// 重新导出文件转录函数
///
/// 这是 STT 模块的核心函数，用于转录音频文件。
/// 详见 [`stt::transcribe_file`] 函数文档。
#[cfg(feature = "stt")]
pub use rs_voice_toolkit_stt::transcribe_file;

/// 重新导出流式转录器
///
/// 用于实时音频流转录的结构体。
/// 详见 [`stt::streaming::StreamingTranscriber`] 结构体文档。
#[cfg(all(feature = "stt", feature = "streaming"))]
pub use rs_voice_toolkit_stt::streaming::StreamingTranscriber;

// 统一错误处理包装函数
#[cfg(feature = "stt")]
mod stt_wrappers {
    use super::*;

    /// 统一错误处理的文件转录函数
    ///
    /// 这是一个包装函数，提供了统一的错误处理接口。它会调用底层的 `stt::transcribe_file`
    /// 函数，并将错误转换为统一的 `Error` 类型。
    ///
    /// ## 参数
    ///
    /// * `model_path` - Whisper 模型文件的路径
    /// * `audio_path` - 要转录的音频文件路径
    ///
    /// ## 返回值
    ///
    /// 返回 `Result<TranscriptionResult, Error>`，其中：
    /// - `Ok(TranscriptionResult)` 包含转录结果，包括文本、时间戳和置信度
    /// - `Err(Error)` 包含错误信息，可能是模型加载错误、音频处理错误等
    ///
    /// ## 示例
    ///
    /// ```ignore
    /// use voice_toolkit::transcribe_file_unified;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let model_path = "models/ggml-base.bin";
    ///     let audio_path = "audio/hello.wav";
    ///     
    ///     match transcribe_file_unified(model_path, audio_path).await {
    ///         Ok(result) => {
    ///             println!("转录结果: {}", result.text);
    ///             println!("处理时间: {:?}", result.processing_time);
    ///         }
    ///         Err(e) => {
    ///             eprintln!("转录失败: {}", e);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    ///
    /// ## 注意事项
    ///
    /// - 首次调用时需要加载模型，可能会有较长的延迟
    /// - 建议在长期运行的应用中保持模型实例以避免重复加载
    /// - 支持多种音频格式，会自动转换为 Whisper 兼容的格式
    /// - 对于大文件，建议使用流式转录功能
    pub async fn transcribe_file_unified<P1, P2>(
        model_path: P1,
        audio_path: P2,
    ) -> Result<crate::stt::TranscriptionResult>
    where
        P1: Into<std::path::PathBuf>,
        P2: AsRef<std::path::Path>,
    {
        crate::stt::transcribe_file(model_path, audio_path)
            .await
            .map_err(Error::from)
    }
}

/// 导出统一错误处理函数
///
/// 这是推荐使用的文件转录函数，提供了统一的错误处理接口。
/// 详见 [`transcribe_file_unified`] 函数文档。
#[cfg(feature = "stt")]
pub use stt_wrappers::transcribe_file_unified;
