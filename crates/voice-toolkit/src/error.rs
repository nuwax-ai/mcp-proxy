//! 统一错误处理模块
//!
//! 该模块为整个语音工具库提供统一的错误处理机制。通过将各个子模块的错误类型
//! 统一封装，使得调用者可以使用单一的错误类型来处理所有可能的错误情况。
//!
//! ## 设计理念
//!
//! - **统一接口**: 所有子模块的错误都转换为统一的 `Error` 枚举
//! - **类型安全**: 使用 `thiserror` 宏确保类型安全的错误处理
//! - **可扩展性**: 支持动态添加新的错误类型
//! - **错误上下文**: 保持原始错误信息，便于调试和错误追踪
//!
//! ## 使用示例
//!
//! ```rust
//! use voice_toolkit::{Error, Result};
//!
//! fn process_audio() -> Result<()> {
//!     // 你的处理逻辑
//!     Ok(())
//! }
//!
//! fn main() {
//!     match process_audio() {
//!         Ok(_) => println!("处理成功"),
//!         Err(Error::Audio(e)) => println!("音频处理错误: {}", e),
//!         Err(Error::Stt(e)) => println!("语音识别错误: {}", e),
//!         Err(Error::Io(e)) => println!("IO错误: {}", e),
//!         Err(Error::Other(e)) => println!("其他错误: {}", e),
//!     }
//! }
//! ```
//!
//! ## 错误类型
//!
//! - `Audio`: 音频处理相关的错误（格式转换、重采样等）
//! - `Stt`: 语音转文本相关的错误（模型加载、转录等）
//! - `Tts`: 文本转语音相关的错误（合成、引擎等）
//! - `Io`: 文件操作相关的错误
//! - `Other`: 其他未分类的错误
//!
//! ## 错误转换
//!
//! 该模块提供了自动的错误转换实现，使得子模块的错误可以自动转换为
//! 统一的错误类型，简化了错误处理代码。

use thiserror::Error;

/// 统一错误类型
///
/// 这是整个语音工具库的主要错误类型，封装了所有可能的错误情况。
/// 使用特性标志来控制不同错误类型的可用性。
#[derive(Error, Debug)]
pub enum Error {
    /// 音频处理错误
    ///
    /// 当音频格式转换、重采样或元数据提取失败时返回此错误。
    /// 需要 `audio` 特性标志。
    ///
    /// # 示例
    ///
    /// ```rust
    /// #[cfg(feature = "audio")]
    /// fn handle_audio_error(err: voice_toolkit::Error) {
    ///     if let voice_toolkit::Error::Audio(audio_err) = err {
    ///         println!("音频处理失败: {}", audio_err);
    ///     }
    /// }
    /// ```
    #[cfg(feature = "audio")]
    #[error("音频错误: {0}")]
    Audio(rs_voice_toolkit_audio::AudioError),

    /// 语音转文本错误
    ///
    /// 当 Whisper 模型加载、文件转录或流式处理失败时返回此错误。
    /// 需要 `stt` 特性标志。
    ///
    /// # 示例
    ///
    /// ```rust
    /// #[cfg(feature = "stt")]
    /// fn handle_stt_error(err: voice_toolkit::Error) {
    ///     if let voice_toolkit::Error::Stt(stt_err) = err {
    ///         println!("语音识别失败: {}", stt_err);
    ///     }
    /// }
    /// ```
    #[cfg(feature = "stt")]
    #[error("语音识别错误: {0}")]
    Stt(rs_voice_toolkit_stt::SttError),

    /// 文本转语音错误
    ///
    /// 当 TTS 引擎初始化、语音合成或输出处理失败时返回此错误。
    /// 需要 `tts` 特性标志。
    ///
    /// # 示例
    ///
    /// ```rust
    /// #[cfg(feature = "tts")]
    /// fn handle_tts_error(err: voice_toolkit::Error) {
    ///     if let voice_toolkit::Error::Tts(tts_err) = err {
    ///         println!("语音合成失败: {}", tts_err);
    ///     }
    /// }
    /// ```
    #[cfg(feature = "tts")]
    #[error("语音合成错误: {0}")]
    Tts(rs_voice_toolkit_tts::TtsError),

    /// IO错误
    ///
    /// 当文件读取、写入或其他 IO 操作失败时返回此错误。
    /// 这是常见的错误类型，通常由文件不存在、权限不足等原因引起。
    ///
    /// # 示例
    ///
    /// ```rust
    /// fn handle_io_error(err: voice_toolkit::Error) {
    ///     if let voice_toolkit::Error::Io(io_err) = err {
    ///         match io_err.kind() {
    ///             std::io::ErrorKind::NotFound => println!("文件不存在"),
    ///             std::io::ErrorKind::PermissionDenied => println!("权限不足"),
    ///             _ => println!("IO错误: {}", io_err),
    ///         }
    ///     }
    /// }
    /// ```
    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),

    /// 其他错误
    ///
    /// 用于处理未分类的其他错误情况。通常用于包装字符串错误消息
    /// 或其他不常见的情况。
    ///
    /// # 示例
    ///
    /// ```rust
    /// fn handle_other_error(err: voice_toolkit::Error) {
    ///     if let voice_toolkit::Error::Other(msg) = err {
    ///         println!("其他错误: {}", msg);
    ///     }
    /// }
    /// ```
    #[error("其他错误: {0}")]
    Other(String),
}

/// 统一结果类型别名
///
/// 这是整个语音工具库的标准结果类型。所有公共函数都返回这个类型，
/// 确保错误处理的一致性。
///
/// # 示例
///
/// ```rust
/// use voice_toolkit::Result;
///
/// fn process_data() -> Result<String> {
///     // 处理逻辑
///     Ok("处理完成".to_string())
/// }
///
/// fn main() {
///     match process_data() {
///         Ok(result) => println!("{}", result),
///         Err(e) => println!("错误: {}", e),
///     }
/// }
/// ```
pub type Result<T> = std::result::Result<T, Error>;

/// 错误辅助函数
impl Error {
    /// 创建其他错误
    ///
    /// 这是一个便利函数，用于创建 `Error::Other` 变体。
    /// 适用于需要从字符串或其他可转换为字符串的类型创建错误的情况。
    ///
    /// # 参数
    ///
    /// * `msg` - 错误消息，可以是任何可转换为 `String` 的类型
    ///
    /// # 返回值
    ///
    /// 返回 `Error::Other` 变体，包含提供的错误消息。
    ///
    /// # 示例
    ///
    /// ```rust
    /// use voice_toolkit::Error;
    ///
    /// fn validate_input(input: &str) -> Result<(), Error> {
    ///     if input.is_empty() {
    ///         return Err(Error::other("输入不能为空"));
    ///     }
    ///     Ok(())
    /// }
    ///
    /// // 也可以直接使用字符串字面量
    /// let error = Error::other("自定义错误消息");
    /// ```
    ///
    /// # 使用场景
    ///
    /// - 验证输入参数
    /// - 业务逻辑错误
    /// - 不适合归类到其他错误类型的情况
    pub fn other<S: Into<String>>(msg: S) -> Self {
        Error::Other(msg.into())
    }
}

/// 从字符串转换
impl From<String> for Error {
    fn from(err: String) -> Self {
        Error::Other(err)
    }
}

/// 从&str转换
impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
}

/// 从STT错误转换
#[cfg(feature = "stt")]
impl From<rs_voice_toolkit_stt::SttError> for Error {
    fn from(err: rs_voice_toolkit_stt::SttError) -> Self {
        Error::Stt(err)
    }
}

/// 从音频错误转换
#[cfg(feature = "audio")]
impl From<rs_voice_toolkit_audio::AudioError> for Error {
    fn from(err: rs_voice_toolkit_audio::AudioError) -> Self {
        Error::Audio(err)
    }
}

/// 从TTS错误转换
#[cfg(feature = "tts")]
impl From<rs_voice_toolkit_tts::TtsError> for Error {
    fn from(err: rs_voice_toolkit_tts::TtsError) -> Self {
        Error::Tts(err)
    }
}
