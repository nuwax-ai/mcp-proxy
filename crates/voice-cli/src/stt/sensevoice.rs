//! SenseVoice ONNX 引擎加载（feature `sensevoice`，transcribe-rs `onnx` 后端）。
//!
//! SenseVoice（阿里 FunASR）非自回归模型：中英日韩粤，**仅批量**（不支持流式），
//! 中文 CER 优于 Whisper、原生输出简体。与 Whisper 同实现 [`transcribe_rs::SpeechModel`]，
//! 故可 `Box<dyn SpeechModel>` 进入与 Whisper 同型的引擎池（见 [`crate::stt::engine_pool`]）。
//!
//! 模型为 sherpa-onnx 目录布局（无自动下载，host 预置）：
//! `{model_dir}/{model.{quant}.onnx, tokens.txt}`，
//! 规范目录名 `sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`。
//! 拉取见 `scripts/dev/fetch-sensevoice-model.sh`。
//!
//! # Send 说明
//! [`transcribe_rs::onnx::sense_voice::SenseVoiceModel`] 持有 `ort::session::Session` 等，
//! ort 的类型均为 `Send`（`SpeechModel: Send` 的实现已证明），故可直接 `Box::new` 为
//! `Box<dyn SpeechModel + Send + 'static>` 入池；若后续 ort 版本变动导致非 Send，
//! 再考虑 `SendWrap`（仅 FFI 场景才用 unsafe，遵循 CLAUDE.md）。

use std::path::{Path, PathBuf};

use transcribe_rs::SpeechModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::onnx::sense_voice::SenseVoiceModel;

use crate::models::config::SenseVoiceConfig;
use crate::stt::error::SttError;

/// SenseVoice 规范模型目录名（sherpa-onnx 发布的 int8 版本）。
pub const DEFAULT_MODEL_DIR_NAME: &str = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17";

/// 解析量化等级字符串 → [`Quantization`]（大小写不敏感）。
pub fn parse_quantization(s: &str) -> Result<Quantization, SttError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "fp32" | "f32" | "" => Ok(Quantization::FP32),
        "fp16" | "f16" | "float16" => Ok(Quantization::FP16),
        "int8" | "i8" => Ok(Quantization::Int8),
        "int4" | "i4" => Ok(Quantization::Int4),
        other => Err(SttError::InvalidInput(format!(
            "未知 SenseVoice 量化等级: {other}（支持 fp32/fp16/int8/int4）"
        ))),
    }
}

/// 解析 SenseVoice 模型目录：配置显式路径优先，否则 `{models_dir}/sensevoice/{DEFAULT_MODEL_DIR_NAME}`。
pub fn resolve_model_dir(models_dir: &str, cfg: &SenseVoiceConfig) -> PathBuf {
    match cfg.model_dir.as_deref() {
        Some(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => PathBuf::from(models_dir)
            .join("sensevoice")
            .join(DEFAULT_MODEL_DIR_NAME),
    }
}

/// 加载 SenseVoice 引擎实例（`Box<dyn SpeechModel>`，与 Whisper 同型入池）。
///
/// `model_dir` 须存在且含 `model.{quant}.onnx` + `tokens.txt`，否则 [`SttError::ModelNotFound`]。
pub fn load(
    model_dir: &Path,
    quantization: &Quantization,
) -> Result<Box<dyn SpeechModel + Send + 'static>, SttError> {
    if !model_dir.exists() {
        return Err(SttError::ModelNotFound {
            model: format!(
                "SenseVoice 模型目录不存在: {}（用 scripts/dev/fetch-sensevoice-model.sh 拉取）",
                model_dir.display()
            ),
        });
    }
    let model = SenseVoiceModel::load(model_dir, quantization).map_err(|e| {
        SttError::InitFailed(format!(
            "SenseVoice 模型加载失败 ({}): {e}",
            model_dir.display()
        ))
    })?;
    tracing::info!(
        "SenseVoice 引擎加载成功: dir={}, quant={:?}",
        model_dir.display(),
        quantization
    );
    Ok(Box::new(model))
}
