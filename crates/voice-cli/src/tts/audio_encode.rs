//! 音频编码：f32 samples → WAV / PCM s16le bytes。
//!
//! WAV = 44 字节 RIFF header + PCM s16le。纯 Rust 实现，零依赖（不依赖 ffmpeg）。
//! MP3 由 ffmpeg-sidecar 转码（P3 暂不暴露 mp3，前端用 wav）。

use crate::tts::error::TtsError;

/// 输出音频格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// WAV（RIFF + PCM s16le，44 字节头）
    Wav,
    /// 裸 PCM s16le（无头）
    PcmS16le,
}

impl AudioFormat {
    /// 从字符串解析（`"wav"` / `"pcm_s16le"`；未知默认 wav）。
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "pcm" | "pcm_s16le" | "raw" => Self::PcmS16le,
            _ => Self::Wav,
        }
    }

    /// HTTP Content-Type。
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Wav => "audio/wav",
            Self::PcmS16le => "audio/pcm",
        }
    }

    /// 文件扩展名。
    pub fn ext(&self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::PcmS16le => "pcm",
        }
    }
}

/// f32 samples（[-1.0, 1.0]）→ s16le bytes（小端 i16）。
pub fn to_pcm_s16le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        // ±1.0 映射到 i16 范围；用 32767 保持对称（避免 +1.0 overflow）
        let v = (clamped * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// f32 samples → WAV bytes（含 44 字节 RIFF 头）。
///
/// `sample_rate` 来自 sherpa-onnx `GeneratedAudio::sample_rate`（Kokoro 通常 24000）。
/// 声道固定 mono（Kokoro 输出单声道）。
pub fn to_wav_bytes(samples: &[f32], sample_rate: i32) -> Result<Vec<u8>, TtsError> {
    if sample_rate <= 0 {
        return Err(TtsError::EncodeFailed(format!("无效采样率：{sample_rate}")));
    }
    let pcm = to_pcm_s16le(samples);
    let data_len = pcm.len() as u32;
    let sample_rate = sample_rate as u32;
    let bits_per_sample: u16 = 16;
    let channels: u16 = 1;
    let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
    let block_align = channels * (bits_per_sample / 8);

    let mut out = Vec::with_capacity(44 + pcm.len());
    // RIFF header
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes()); // chunk size
    out.extend_from_slice(b"WAVE");
    // fmt subchunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM = 1
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    // data subchunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(&pcm);
    Ok(out)
}

/// 按格式编码。
pub fn encode(samples: &[f32], sample_rate: i32, fmt: AudioFormat) -> Result<Vec<u8>, TtsError> {
    match fmt {
        AudioFormat::Wav => to_wav_bytes(samples, sample_rate),
        AudioFormat::PcmS16le => Ok(to_pcm_s16le(samples)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_s16le_basic() {
        let v = to_pcm_s16le(&[0.0, 1.0, -1.0, 0.5]);
        // 4 samples × 2 bytes = 8 bytes
        assert_eq!(v.len(), 8);
        // 0.0 → 0
        assert_eq!(i16::from_le_bytes([v[0], v[1]]), 0);
        // 1.0 → 32767
        assert_eq!(i16::from_le_bytes([v[2], v[3]]), 32767);
        // -1.0 → -32767
        assert_eq!(i16::from_le_bytes([v[4], v[5]]), -32767);
    }

    #[test]
    fn pcm_clamps_overflow() {
        // 超出 [-1,1] 必须 clamp，不能 overflow
        let v = to_pcm_s16le(&[2.0, -2.0]);
        assert_eq!(i16::from_le_bytes([v[0], v[1]]), 32767);
        assert_eq!(i16::from_le_bytes([v[2], v[3]]), -32767);
    }

    #[test]
    fn wav_header_correct() {
        let wav = to_wav_bytes(&[0.0; 24000], 24000).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        // data_len = 24000 samples × 2 bytes
        let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]);
        assert_eq!(data_len, 48000);
        // total = 44 + 48000
        assert_eq!(wav.len(), 44 + 48000);
    }

    #[test]
    fn wav_rejects_bad_sample_rate() {
        assert!(to_wav_bytes(&[0.0], 0).is_err());
        assert!(to_wav_bytes(&[0.0], -1).is_err());
    }

    #[test]
    fn format_parse() {
        assert_eq!(AudioFormat::parse("wav"), AudioFormat::Wav);
        assert_eq!(AudioFormat::parse("WAV"), AudioFormat::Wav);
        assert_eq!(AudioFormat::parse("pcm_s16le"), AudioFormat::PcmS16le);
        assert_eq!(AudioFormat::parse("pcm"), AudioFormat::PcmS16le);
        // 未知 → wav
        assert_eq!(AudioFormat::parse("mp3"), AudioFormat::Wav);
    }
}
