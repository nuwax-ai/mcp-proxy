//! STT模块的错误处理
//!
//! 定义了语音转文本过程中可能出现的各种错误类型

use thiserror::Error;

/// STT模块的主要错误类型
#[derive(Error, Debug)]
pub enum SttError {
    /// 音频文件相关错误
    #[error("音频文件错误: {0}")]
    AudioFileError(String),

    /// 文件未找到错误
    #[error("文件未找到: {0}")]
    FileNotFound(String),

    /// 音频格式不支持
    #[error("不支持的音频格式: {0}")]
    UnsupportedFormat(String),

    /// Whisper模型相关错误
    #[error("Whisper模型错误: {0}")]
    WhisperError(String),

    /// 模型加载失败
    #[error("模型加载失败: {0}")]
    ModelLoadError(String),

    /// 转录失败
    #[error("转录失败: {0}")]
    TranscriptionError(String),

    /// 音频处理错误
    #[error("音频处理错误: {0}")]
    AudioProcessingError(String),

    /// 音频重采样错误
    #[error("音频重采样错误: {0}")]
    ResamplingError(String),

    /// 流处理错误
    #[error("流处理错误: {0}")]
    StreamError(String),

    /// 配置错误
    #[error("配置错误: {0}")]
    ConfigError(String),

    /// IO错误
    #[error("IO错误: {0}")]
    IoError(#[from] std::io::Error),

    /// 其他错误
    #[error("其他错误: {0}")]
    Other(String),
}

/// STT结果类型别名
pub type SttResult<T> = Result<T, SttError>;

/// 从whisper-rs错误转换
impl From<whisper_rs::WhisperError> for SttError {
    fn from(err: whisper_rs::WhisperError) -> Self {
        SttError::WhisperError(err.to_string())
    }
}

/// 从hound错误转换
impl From<hound::Error> for SttError {
    fn from(err: hound::Error) -> Self {
        SttError::AudioFileError(err.to_string())
    }
}

/// 从rubato错误转换
impl From<rubato::ResampleError> for SttError {
    fn from(err: rubato::ResampleError) -> Self {
        SttError::ResamplingError(err.to_string())
    }
}

// 注意：如需从外部音频库错误转换，请在调用处进行显式映射，避免耦合具体错误类型

/// 错误辅助函数
impl SttError {
    /// 创建文件未找到错误
    pub fn file_not_found<S: Into<String>>(path: S) -> Self {
        SttError::FileNotFound(path.into())
    }

    /// 创建不支持格式错误
    pub fn unsupported_format<S: Into<String>>(format: S) -> Self {
        SttError::UnsupportedFormat(format.into())
    }

    /// 创建模型加载错误
    pub fn model_load_error<S: Into<String>>(msg: S) -> Self {
        SttError::ModelLoadError(msg.into())
    }

    /// 创建转录错误
    pub fn transcription_error<S: Into<String>>(msg: S) -> Self {
        SttError::TranscriptionError(msg.into())
    }

    /// 创建配置错误
    pub fn config_error<S: Into<String>>(msg: S) -> Self {
        SttError::ConfigError(msg.into())
    }

    /// 创建其他错误
    pub fn other<S: Into<String>>(msg: S) -> Self {
        SttError::Other(msg.into())
    }
}
