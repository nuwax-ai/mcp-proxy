//! TTS 合成：封装 sherpa-onnx `OfflineTts::generate_with_config`。
//!
//! 职责：
//! - 文本预清洗（**含 NUL 会 panic**：sherpa-onnx 内部 `CString::new(text).unwrap()` at tts.rs:567）
//! - 把 `Option<GeneratedAudio>` 错误映射成 `TtsError::SynthFailed`（带上下文）
//! - **必须 `samples.to_vec()`**：`GeneratedAudio` 借用 C 内存且 `!Send`，跨 await / spawn_blocking
//!   之前必须拷贝出所有权数据
//!
//! [`Synthesizer`] trait 抽象引擎（对称 STT `Decoder` trait），使 [`crate::tts::synthesize_streaming`]
//! 的 callback/event 编排可用 `MockSynthesizer` 单测，不必拉起真实 sherpa-onnx。

use sherpa_onnx::{GeneratedAudio, GenerationConfig, OfflineTts};

use crate::tts::error::TtsError;
use crate::tts::options::TtsOptions;

/// 合成引擎抽象（对称 STT `stt::Decoder` trait）。
///
/// 生产实现 `OfflineTts`；测试用 `MockSynthesizer` 验证 streaming 的 callback/event 编排。
/// trait 方法 `generate` 返回总样本数（非 `GeneratedAudio`——后者持 C 指针不可在测试构造）。
pub trait Synthesizer: Send + Sync {
    /// 输出采样率（Hz）
    fn sample_rate(&self) -> i32;
    /// 合成：对每个样本 chunk 调 `callback(samples, progress)`；callback 返回 `false` 中断。
    /// 返回总样本数（用于 Done 事件）；`None` = 失败/取消。
    fn generate<F>(&self, text: &str, cfg: &GenerationConfig, callback: Option<F>) -> Option<usize>
    where
        F: FnMut(&[f32], f32) -> bool + 'static;
}

impl Synthesizer for OfflineTts {
    fn sample_rate(&self) -> i32 {
        OfflineTts::sample_rate(self)
    }
    fn generate<F>(&self, text: &str, cfg: &GenerationConfig, callback: Option<F>) -> Option<usize>
    where
        F: FnMut(&[f32], f32) -> bool + 'static,
    {
        self.generate_with_config(text, cfg, callback)
            .map(|a| a.samples().len())
    }
}

/// 合成结果（所有权数据，Send 安全）。
#[derive(Debug, Clone)]
pub struct SynthesizedAudio {
    /// f32 PCM samples，归一化到 [-1.0, 1.0]
    pub samples: Vec<f32>,
    /// 采样率（Hz，Kokoro 通常 24000）
    pub sample_rate: i32,
}

/// 同步合成（在 `spawn_blocking` 中调用）。
///
/// `tts` 来自引擎池（`Arc<Mutex<OfflineTts>>` 的 guard）；`opts` 为 per-request 参数。
/// callback 流式增量留给 P5（`tts/streaming.rs`），这里 `None` 一次性整段合成。
pub fn synthesize(
    tts: &OfflineTts,
    text: &str,
    opts: &TtsOptions,
) -> Result<SynthesizedAudio, TtsError> {
    // Fail Fast：参数范围校验
    opts.validate()?;

    // 预清洗 interior NUL（sherpa-onnx CString::new 会 panic）
    let clean_text = sanitize_text(text);

    let gen_cfg = opts.to_generation_config();
    // callback=None：一次性整段合成（流式增量合成留给 P5）。turbofish 消除 F 类型歧义。
    let audio = tts
        .generate_with_config::<fn(&[f32], f32) -> bool>(&clean_text, &gen_cfg, None)
        .ok_or_else(|| {
            TtsError::SynthFailed(format!(
                "generate_with_config 返回 None（text 长度 {}，sid={}）。\
                 常见原因：文本为空 / 全是 unsupported 字符 / 模型与 voices.bin 不匹配",
                clean_text.chars().count(),
                opts.sid
            ))
        })?;

    Ok(OwnedAudio::from(&audio).into())
}

/// 清洗文本：去 interior NUL + trim 首尾空白。
///
/// sherpa-onnx 的 `CString::new(text)` 遇到 interior NUL 会 panic（unwrap at tts.rs:567），
/// 故必须先剔除。其余字符保留（含标点、换行——Kokoro 自行处理）。
fn sanitize_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.contains('\0') {
        trimmed.replace('\0', "")
    } else {
        trimmed.to_string()
    }
}

/// 从 `GeneratedAudio` 拷贝出所有权数据（绕开 `!Send`）。
///
/// `GeneratedAudio` 持有 C 端指针，`Drop` 时调 C 释放；我们必须在其 drop 前拷贝 samples。
struct OwnedAudio {
    samples: Vec<f32>,
    sample_rate: i32,
}

impl From<&GeneratedAudio> for OwnedAudio {
    fn from(audio: &GeneratedAudio) -> Self {
        Self {
            samples: audio.samples().to_vec(),
            sample_rate: audio.sample_rate(),
        }
    }
}

impl From<OwnedAudio> for SynthesizedAudio {
    fn from(o: OwnedAudio) -> Self {
        Self {
            samples: o.samples,
            sample_rate: o.sample_rate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_removes_nul() {
        assert_eq!(sanitize_text("he\0llo"), "hello");
        assert_eq!(sanitize_text("  hi  "), "hi");
        assert_eq!(sanitize_text("你好\0世界"), "你好世界");
    }

    #[test]
    fn sanitize_keeps_punctuation_and_newline() {
        assert_eq!(sanitize_text("你好，世界！"), "你好，世界！");
        assert_eq!(sanitize_text("a\nb"), "a\nb");
    }
}
