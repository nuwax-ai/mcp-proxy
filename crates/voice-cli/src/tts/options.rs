//! TTS 合成参数 DTO，映射到 sherpa-onnx 的 `GenerationConfig`（per-request）。
//!
//! model-level 参数（`length_scale` / `noise_scale`）走 `OfflineTtsKokoroModelConfig`，
//! 在 `TtsLoadParams` + `engine_pool::build_tts` 中设置，不在本 DTO 内（避免
//! "接受 per-request 输入但只能 model-level 生效"的静默忽略——Fail Fast）。

use sherpa_onnx::GenerationConfig;

/// voice-cli 侧的 TTS 合成参数（per-request）。
///
/// 所有字段都真正 per-request，经 [`TtsOptions::to_generation_config`] 进入
/// sherpa-onnx `GenerationConfig`（注意 sherpa-onnx 用 `sid` 不是 `speaker_id`）。
///
/// `length_scale` 等 model-level 参数**故意不在本结构**：它们只在引擎首次加载时
/// 生效（池化实例共享），放在 per-request DTO 会让客户端误以为可逐请求调整。
/// 走 `TtsLoadParams.length_scale`（来自 `config.tts.engine.default_length_scale`）。
///
/// `noise_scale` / `noise_scale_w` 是 VITS 专属，Kokoro 不用；首版只支持 Kokoro。
#[derive(Debug, Clone)]
pub struct TtsOptions {
    /// 音色 id（Kokoro voices.bin 的多 speaker 索引，从 0 开始）
    pub sid: i32,
    /// 语速（1.0 = 原速；>1 加速，<1 减速）
    pub speed: f32,
    /// 句间静音缩放（sherpa-onnx 默认 0.2）
    pub silence_scale: f32,
}

impl Default for TtsOptions {
    fn default() -> Self {
        Self {
            sid: 0,
            speed: 1.0,
            silence_scale: 0.2,
        }
    }
}

impl TtsOptions {
    /// 校验参数范围（Fail Fast：越界立即报错，避免传到 C 端产生不可预期的合成）。
    pub fn validate(&self) -> Result<(), crate::tts::TtsError> {
        if self.speed <= 0.0 || !self.speed.is_finite() {
            return Err(crate::tts::TtsError::InvalidInput(format!(
                "speed 必须为正数，收到 {}",
                self.speed
            )));
        }
        if self.silence_scale < 0.0 || !self.silence_scale.is_finite() {
            return Err(crate::tts::TtsError::InvalidInput(format!(
                "silence_scale 不能为负，收到 {}",
                self.silence_scale
            )));
        }
        if self.sid < 0 {
            return Err(crate::tts::TtsError::InvalidInput(format!(
                "sid 不能为负，收到 {}",
                self.sid
            )));
        }
        Ok(())
    }

    /// 映射到 sherpa-onnx `GenerationConfig`（per-request 部分）。
    pub fn to_generation_config(&self) -> GenerationConfig {
        GenerationConfig {
            sid: self.sid,
            speed: self.speed,
            silence_scale: self.silence_scale,
            ..Default::default()
        }
    }
}
