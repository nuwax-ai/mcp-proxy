//! 统一错误处理模块（audio-only）
//!
//! STT/TTS 错误已由 voice-cli 自有的 thiserror 类型承载，本 crate 仅封装 audio 模块错误。

use thiserror::Error;

/// 统一错误类型
#[derive(Error, Debug)]
pub enum Error {
    /// 音频处理错误（格式转换、重采样、元数据提取）
    #[cfg(feature = "audio")]
    #[error("音频错误: {0}")]
    Audio(rs_voice_toolkit_audio::AudioError),

    /// IO 错误
    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),

    /// 其他未分类错误
    #[error("其他错误: {0}")]
    Other(String),
}

/// 统一结果类型别名
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// 从字符串创建 `Error::Other`
    pub fn other<S: Into<String>>(msg: S) -> Self {
        Error::Other(msg.into())
    }
}

impl From<String> for Error {
    fn from(err: String) -> Self {
        Error::Other(err)
    }
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
}

#[cfg(feature = "audio")]
impl From<rs_voice_toolkit_audio::AudioError> for Error {
    fn from(err: rs_voice_toolkit_audio::AudioError) -> Self {
        Error::Audio(err)
    }
}
