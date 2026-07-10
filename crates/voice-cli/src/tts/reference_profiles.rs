//! ZipVoice reference 音色管理（预置 profile 缓存 + 动态 base64 WAV 解码）。
//!
//! ZipVoice 是零样本**克隆**引擎：每次合成需 `reference_audio`(f32 PCM) +
//! `reference_text` + `reference_sample_rate` 决定音色。
//!
//! **双轨**（对齐 plan 决策 4）：
//! - **预置**：`config.tts.engine.zipvoice.voices`，启动期 [`init`] 加载 WAV 缓存；
//!   请求 `voice:"{name}"` → [`lookup`]。
//! - **动态**：请求期带 `reference_audio`(base64 WAV) → [`from_base64_wav`] 即时克隆。
//!
//! WAV 解码复用 sherpa-onnx 自带的 [`sherpa_onnx::Wave::read`]（官方 API，纯 WAV I/O，
//! 不涉 onnxruntime → 无 Mac SIGKILL 风险）。动态 base64 先写 tempfile 再读。
//!
//! 类比 [`crate::stt::sherpa_punc`] 的 `OnceLock` 全局 init 模式（Fail Fast）。

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sherpa_onnx::Wave;

use crate::models::config::ZipVoiceProfile;
use crate::tts::TtsError;

/// ZipVoice reference（所有权数据，`Send` 安全；合成时填进 `GenerationConfig`）。
#[derive(Debug, Clone)]
pub struct ZipVoiceRef {
    /// f32 PCM samples，归一化 [-1.0, 1.0]
    pub samples: Vec<f32>,
    /// 采样率（Hz）
    pub sample_rate: i32,
    /// 参考音频的精确转写文本
    pub text: String,
}

/// 预置音色 registry（启动期 [`init`]；name → 缓存的 reference）。
static PROFILES: OnceLock<HashMap<String, Arc<ZipVoiceRef>>> = OnceLock::new();

/// config 顺序的首个预置音色名（resolve_reference 无 voice/reference_audio 时的**确定性**回退；
/// 避免 HashMap 迭代随机性导致回退音色不稳定 —— Heisenbug）。
static FIRST_PROFILE: OnceLock<Option<String>> = OnceLock::new();

/// 启动期加载预置音色组（Fail Fast：任一 profile 文件缺 / WAV 解码失败即返回错误）。
///
/// 幂等（已 init 直接 `Ok(())`）。`profiles` 为空也合法（纯动态克隆模式）。
pub fn init(profiles: &[ZipVoiceProfile]) -> Result<(), TtsError> {
    if PROFILES.get().is_some() {
        return Ok(());
    }
    let mut map = HashMap::with_capacity(profiles.len());
    for p in profiles {
        // ZipVoice C++ 强校验 reference_text 非空 → 启动期 Fail Fast（配置错误不应静默到合成时才暴露）
        let text = p.reference_text.trim();
        if text.is_empty() {
            return Err(TtsError::ModelNotFound {
                model: format!(
                    "ZipVoice 预置音色 {} 的 reference_text 为空（须为参考音频的精确转写）",
                    p.name
                ),
            });
        }
        let r = load_wav_file(&p.reference_wav, text).map_err(|e| TtsError::ModelNotFound {
            model: format!("ZipVoice 预置音色 {}: {e}", p.name),
        })?;
        tracing::info!(
            "ZipVoice 预置音色已加载: {} ({} samples, {}Hz)",
            p.name,
            r.samples.len(),
            r.sample_rate
        );
        map.insert(p.name.clone(), Arc::new(r));
    }
    // 记录 config 顺序首个 profile（resolve_reference 回退用，确定性）
    let first = profiles.first().map(|p| p.name.clone());
    // OnceLock::set 仅在“并发两线程同时首次 init”时返回 Err（当前单调用点不会发生，但代码保持健壮）：
    // 失败=另一线程已 set 同源 config 的等价 map，丢弃本 map 无害（幂等）。
    if PROFILES.set(map).is_err() {
        tracing::debug!("reference_profiles: 并发 init，丢弃重复 map（幂等）");
    }
    let _ = FIRST_PROFILE.set(first);
    Ok(())
}

/// 按名查预置音色（未 init 或 name 不存在 → `None`）。
pub fn lookup(name: &str) -> Option<Arc<ZipVoiceRef>> {
    PROFILES.get().and_then(|m| m.get(name).cloned())
}

/// 预置音色名列表（排序后返回，供 `/api/v1/tts/voices` 展示稳定，避免 HashMap 随机顺序）。
pub fn profile_names() -> Vec<String> {
    let mut names: Vec<String> = PROFILES
        .get()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// 解析 ZipVoice reference（优先级：`reference_audio` 动态 > `voice` 预置 > 回退首个预置 > Err）。
///
/// backend=zipvoice 时调用；Kokoro 不用（调用方按 backend 分支，Kokoro 直接 None）。
/// 动态 base64 解码是 CPU/IO 密集，调用方应在 `spawn_blocking` 中调用。
pub fn resolve_reference(
    voice: Option<&str>,
    reference_audio: Option<&str>,
    reference_text: Option<&str>,
) -> Result<Option<Arc<ZipVoiceRef>>, TtsError> {
    if let Some(b64) = reference_audio.filter(|s| !s.trim().is_empty()) {
        // ZipVoice C++ 强校验 reference_text 非空（空则静默返回空音频）→ 应用层 Fail Fast 拦截
        let text = reference_text
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                TtsError::InvalidInput(
                    "动态克隆需同时提供非空 reference_text（参考音频的精确转写文本）".into(),
                )
            })?;
        return from_base64_wav(b64, text).map(|r| Some(Arc::new(r)));
    }
    if let Some(name) = voice.filter(|s| !s.is_empty()) {
        return lookup(name)
            .ok_or_else(|| TtsError::InvalidInput(format!("ZipVoice 预置音色不存在: {name}")))
            .map(Some);
    }
    // 回退 config 顺序的首个预置 profile（确定性，避免 HashMap 迭代随机性）
    if let Some(first) = FIRST_PROFILE.get().and_then(|o| o.as_ref()) {
        return lookup(first)
            .ok_or_else(|| TtsError::InvalidInput("ZipVoice 预置 registry 未初始化".into()))
            .map(Some);
    }
    Err(TtsError::InvalidInput(
        "backend=zipvoice 需提供 voice（预置音色名）或 reference_audio（base64 WAV 动态克隆）"
            .into(),
    ))
}

/// 动态解码 base64 WAV → [`ZipVoiceRef`]（请求期；不缓存）。
///
/// 用于克隆完全体：任意音频即时克隆。`text` 须为该音频的精确转写。
/// base64 → bytes → tempfile → `Wave::read`（Wave 只接受路径）。
pub fn from_base64_wav(b64: &str, text: &str) -> Result<ZipVoiceRef, TtsError> {
    let trimmed = b64.trim();
    if trimmed.is_empty() {
        return Err(TtsError::InvalidInput(
            "reference_audio 为空（backend=zipvoice 需 reference_audio 或 voice）".into(),
        ));
    }
    let bytes = BASE64
        .decode(trimmed)
        .map_err(|e| TtsError::InvalidInput(format!("reference_audio base64 解码失败: {e}")))?;
    // tempfile：Wave::read 只接受文件路径；NamedTempFile drop 时自动删
    let tmp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .map_err(|e| TtsError::InvalidInput(format!("创建临时 WAV 失败: {e}")))?;
    std::fs::write(tmp.path(), &bytes)
        .map_err(|e| TtsError::InvalidInput(format!("写临时 WAV 失败: {e}")))?;
    load_wav_file(&tmp.path().to_string_lossy(), text)
}

/// 读 WAV 文件 → [`ZipVoiceRef`]（预置 init + 动态 tempfile 共用）。
///
/// 用 sherpa-onnx `Wave::read`（归一化 f32；纯 WAV I/O，不涉 onnxruntime）。
fn load_wav_file(path: &str, text: &str) -> Result<ZipVoiceRef, TtsError> {
    let wave = Wave::read(path).ok_or_else(|| {
        TtsError::InvalidInput(format!("WAV 解码失败（{path}）：文件不存在/损坏/非 WAV"))
    })?;
    let samples = wave.samples().to_vec();
    if samples.is_empty() {
        return Err(TtsError::InvalidInput(format!(
            "WAV 解码出 0 样本（{path}）：文件损坏或格式不支持"
        )));
    }
    Ok(ZipVoiceRef {
        samples,
        sample_rate: wave.sample_rate(),
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小合法 WAV（mono s16le PCM，n 样本正弦）的字节数组。
    fn minimal_wav_bytes(sample_rate: u32, n: u32) -> Vec<u8> {
        let n_samples = n;
        let data_len = n_samples * 2; // s16le
        let mut out = Vec::with_capacity(44 + data_len as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVE");
        out.extend_from_slice(b"fmt ");
        out.extend_from_slice(&16u32.to_le_bytes()); // subchunk size
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&sample_rate.to_le_bytes());
        out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        out.extend_from_slice(&2u16.to_le_bytes()); // block align
        out.extend_from_slice(&16u16.to_le_bytes()); // bits
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..n_samples {
            let v = ((i as f32 * 0.1).sin() * 16000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    #[test]
    fn from_base64_wav_empty_rejected() {
        let err = from_base64_wav("   ", "t").unwrap_err();
        assert!(matches!(err, TtsError::InvalidInput(_)));
    }

    #[test]
    fn from_base64_wav_garbage_rejected() {
        // 非 base64 / 非 WAV → 解码或 Wave::read 失败
        let err = from_base64_wav("@@@不是合法base64@@@", "t").unwrap_err();
        assert!(matches!(err, TtsError::InvalidInput(_)));
    }

    #[test]
    fn from_base64_wav_roundtrip() {
        let bytes = minimal_wav_bytes(16000, 200);
        let b64 = BASE64.encode(&bytes);
        let r = from_base64_wav(&b64, "参考文本").unwrap();
        assert_eq!(r.sample_rate, 16000);
        assert_eq!(r.samples.len(), 200);
        assert!(r.samples.iter().all(|s| s.abs() <= 1.0));
        assert_eq!(r.text, "参考文本");
    }

    #[test]
    fn resolve_reference_err_when_nothing_provided() {
        // 未 init registry + 无 voice + 无 reference_audio → Err
        let err = resolve_reference(None, None, None).unwrap_err();
        assert!(matches!(err, TtsError::InvalidInput(_)));
    }

    #[test]
    fn resolve_reference_dynamic_requires_nonempty_text() {
        // 动态克隆：reference_audio 提供但 reference_text 空/空白 → Fail Fast
        // （ZipVoice C++ 强校验 reference_text 非空，空则静默返回空音频，故应用层提前拦截）
        let err = resolve_reference(None, Some("dummy_b64"), None).unwrap_err();
        assert!(
            err.to_string().contains("reference_text"),
            "应提示 reference_text 缺失，实际: {err}"
        );
        let err2 = resolve_reference(None, Some("dummy_b64"), Some("   ")).unwrap_err();
        assert!(err2.to_string().contains("reference_text"));
    }
}
