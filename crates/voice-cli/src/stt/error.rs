//! STT 子系统错误类型。

use thiserror::Error;

/// STT（语音转文字）错误。
///
/// 设计遵循 Fail Fast + 上下文丰富：每个变体带足够信息定位问题。
/// 调用方（HTTP handler）按变体映射 HTTP 状态码：
/// - `ModelNotFound` / `InvalidInput` → 400（客户端可修）
/// - `InitFailed` / `InferFailed` / `Audio` → 500（服务端问题）
/// - `Cancelled` / `Timeout` → 499 / 504
#[derive(Debug, Error)]
pub enum SttError {
    #[error("STT 模型未找到: {model}")]
    ModelNotFound { model: String },

    #[error("STT 引擎初始化失败: {0}")]
    InitFailed(String),

    #[error("STT 推理失败: {0}")]
    InferFailed(String),

    #[error("STT 音频处理失败: {0}")]
    Audio(String),

    #[error("STT 任务被取消")]
    Cancelled,

    #[error("STT 任务超时（{secs}s）")]
    Timeout { secs: u64 },

    #[error("STT 无效输入: {0}")]
    InvalidInput(String),
}

/// 把 transcribe-rs 的错误映射到 SttError。
///
/// - `ModelNotFound` 保留语义（路径 → 字符串）
/// - `Inference` → `InferFailed`
/// - `Audio` → `Audio`
/// - 其它（Config / Io / Other）统一归 `InferFailed`（带 to_string）
impl From<transcribe_rs::TranscribeError> for SttError {
    fn from(e: transcribe_rs::TranscribeError) -> Self {
        use transcribe_rs::TranscribeError as E;
        match e {
            E::ModelNotFound(p) => SttError::ModelNotFound {
                model: p.display().to_string(),
            },
            E::Inference(m) => SttError::InferFailed(m),
            E::Audio(m) => SttError::Audio(m),
            other => SttError::InferFailed(other.to_string()),
        }
    }
}
