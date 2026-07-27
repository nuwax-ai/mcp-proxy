//! sherpa-onnx 标点恢复（`OfflinePunctuation`，CT-Transformer）。
//!
//! **仅用于 FireRedASR2-AED**（AED 无标点输出）；Fun-ASR-Nano / Qwen3-ASR 自带 LLM 标点，
//! 调用方按 kind 判断后调用 [`add_punct`]，**不重复加**（否则双标点）。
//!
//! 全局 `OnceLock<Option<OfflinePunctuation>>`，启动时 [`init`] 一次（punct=true 且模型存在）；
//! 未加载 / 加失败 → [`add_punct`] 原样透传（Fail-Soft，不阻塞 STT）。

use std::path::Path;
use std::sync::OnceLock;

use sherpa_onnx::{OfflinePunctuation, OfflinePunctuationConfig, OfflinePunctuationModelConfig};

/// 默认标点模型目录名（sherpa-onnx `punctuation-models` release）。
pub const DEFAULT_PUNCT_DIR: &str = "sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12";
/// 标点模型文件名（ct_transformer 指向它）。
pub const PUNCT_MODEL_FILE: &str = "model.onnx";

/// 进程级标点恢复器（None = 未启用/加载失败，透传）。
static PUNC: OnceLock<Option<OfflinePunctuation>> = OnceLock::new();

/// 启动时调用一次：`punct=true` 且 `model_onnx` 存在 → 加载；否则置 None（透传）。幂等。
///
/// `model_onnx` 是标点模型的 `model.onnx` 全路径。`provider` 透传 sherpa EP（Mac coreml / Linux cuda）。
pub fn init(model_onnx: Option<&Path>, provider: Option<&str>, num_threads: i32, debug: bool) {
    if PUNC.get().is_some() {
        return;
    }
    let loaded = match model_onnx {
        Some(p) if p.is_file() => {
            let cfg = OfflinePunctuationConfig {
                model: OfflinePunctuationModelConfig {
                    ct_transformer: Some(p.to_string_lossy().into_owned()),
                    num_threads,
                    debug,
                    provider: provider.map(|s| s.to_string()),
                },
            };
            match OfflinePunctuation::create(&cfg) {
                Some(m) => {
                    tracing::info!("标点恢复模型已加载: {}", p.display());
                    Some(m)
                }
                None => {
                    tracing::warn!(
                        "OfflinePunctuation::create 返回 None（{}），标点透传。常见原因：模型损坏 / onnxruntime 版本不匹配",
                        p.display()
                    );
                    None
                }
            }
        }
        Some(p) => {
            tracing::warn!("标点模型不存在: {}，标点透传", p.display());
            None
        }
        None => None, // 未配置 → 透传（调用方可能 punct=false 或仅给非 AED 引擎用）
    };
    let _ = PUNC.set(loaded);
}

/// 给文本加标点；标点器未加载 / 加失败（如内嵌 NUL）→ 原样透传（Fail-Soft）。
pub fn add_punct(text: &str) -> String {
    match PUNC.get() {
        Some(Some(p)) => p.add_punctuation(text).unwrap_or_else(|| text.to_string()),
        _ => text.to_string(),
    }
}
