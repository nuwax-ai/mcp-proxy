//! STT 转录参数 DTO，映射到 transcribe-rs 的 `WhisperInferenceParams`。

use transcribe_rs::whisper_cpp::WhisperInferenceParams;

/// voice-cli 侧的 STT 转录参数。
///
/// # 注意
/// transcribe-rs 的 WhisperEngine 内部用 `BeamSearch`（`beam_size = 3` 硬编码），
/// 不暴露 `temperature` / `beam_size`。故本结构暂不含这两项；
/// 未来若切换 `SamplingStrategy`（如 Greedy）再扩展。
#[derive(Debug, Clone)]
pub struct SttTranscribeOptions {
    /// 目标语种（BCP-47，如 `"en"` / `"zh"`）。`None` = 自动检测
    pub language: Option<String>,
    /// 初始提示，给模型领域上下文（提升专有词 / 风格准确率）
    pub initial_prompt: Option<String>,
    /// 是否翻译为英文（仅多语言模型）
    pub translate: bool,
    /// 抑制非语音 token（默认 true）
    pub suppress_non_speech_tokens: bool,
    /// 无语音阈值 0.0–1.0（默认 0.2）
    pub no_speech_thold: f32,
    /// 解码线程数（0 = whisper.cpp 默认 `min(4, num_cores)`）
    pub n_threads: i32,
}

impl Default for SttTranscribeOptions {
    fn default() -> Self {
        Self {
            language: None,
            initial_prompt: None,
            translate: false,
            suppress_non_speech_tokens: true,
            no_speech_thold: 0.2,
            n_threads: 0,
        }
    }
}

impl SttTranscribeOptions {
    /// 映射到 transcribe-rs `WhisperInferenceParams`。
    ///
    /// 注意：transcribe-rs **0.3.11** 的 `WhisperInferenceParams` 尚未暴露 `no_context` 字段
    ///（本地 git master 已加，但未发布到 crates.io）。0.3.11 单次 `transcribe_with` 走
    /// whisper.cpp 默认上下文策略，对批量转录（单次解码）无影响；P2 LocalAgreement 双解码
    /// 若需强制无状态，再评估升级 transcribe-rs 版本。
    pub fn to_inference_params(&self) -> WhisperInferenceParams {
        WhisperInferenceParams {
            language: self.language.clone(),
            translate: self.translate,
            suppress_non_speech_tokens: self.suppress_non_speech_tokens,
            no_speech_thold: self.no_speech_thold,
            n_threads: self.n_threads,
            initial_prompt: self.initial_prompt.clone(),
            ..Default::default()
        }
    }
}
