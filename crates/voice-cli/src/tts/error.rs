//! TTS 子系统错误类型。

use thiserror::Error;

/// TTS（文本转语音）错误。
///
/// 设计遵循 Fail Fast + 上下文丰富：每个变体带足够信息定位问题。
/// 调用方（HTTP handler）按变体映射 HTTP 状态码：
/// - `ModelNotFound` / `InvalidInput` → 400（客户端可修）
/// - `InitFailed` / `SynthFailed` / `EncodeFailed` → 500（服务端问题）
/// - `Cancelled` / `Timeout` → 499 / 504
///
/// 注意：sherpa-onnx 的 `OfflineTts::create` / `generate_with_config` 返回 `Option`
/// 而非 `Result`（C 端错误打 stderr 拿不到），由 [`crate::tts::engine_pool`] /
/// [`crate::tts::synthesizer`] 包装成这里的强类型错误（带模型 / 路径上下文）。
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("TTS 模型未找到: {model}")]
    ModelNotFound { model: String },

    #[error("TTS 引擎初始化失败: {0}")]
    InitFailed(String),

    #[error("TTS 合成失败: {0}")]
    SynthFailed(String),

    #[error("TTS 音频编码失败: {0}")]
    EncodeFailed(String),

    #[error("TTS 任务被取消")]
    Cancelled,

    #[error("TTS 任务超时（{secs}s）")]
    Timeout { secs: u64 },

    #[error("TTS 无效输入: {0}")]
    InvalidInput(String),
}
