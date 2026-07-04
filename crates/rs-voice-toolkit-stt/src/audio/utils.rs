//! 音频处理工具函数
//!
//! 提供音频文件读写、格式检测、数据转换等辅助功能

use super::{AudioConfig, AudioFormat};
use crate::error::{SttError, SttResult};
use log::{info, warn};
use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

/// 音频数据结构
#[derive(Debug, Clone)]
pub struct AudioData {
    /// 音频样本数据（f32格式，范围 -1.0 到 1.0）
    pub samples: Vec<f32>,
    /// 音频配置
    pub config: AudioConfig,
}

impl AudioData {
    /// 创建新的音频数据
    pub fn new(samples: Vec<f32>, config: AudioConfig) -> Self {
        Self { samples, config }
    }

    /// 获取音频时长（秒）
    pub fn duration(&self) -> f64 {
        let frames = self.samples.len() / self.config.channels as usize;
        frames as f64 / self.config.sample_rate as f64
    }

    /// 获取帧数
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.config.channels as usize
    }

    /// 转换为单声道
    pub fn to_mono(&self) -> AudioData {
        if self.config.channels == 1 {
            return self.clone();
        }

        let frame_count = self.frame_count();
        let mut mono_samples = Vec::with_capacity(frame_count);

        for frame in 0..frame_count {
            let mut sum = 0.0;
            for channel in 0..self.config.channels {
                let index = frame * self.config.channels as usize + channel as usize;
                if index < self.samples.len() {
                    sum += self.samples[index];
                }
            }
            mono_samples.push(sum / self.config.channels as f32);
        }

        let mono_config = AudioConfig {
            channels: 1,
            ..self.config
        };

        AudioData::new(mono_samples, mono_config)
    }

    /// 标准化音频音量
    pub fn normalize(&mut self) {
        if self.samples.is_empty() {
            return;
        }

        // 找到最大绝对值
        let max_abs = self.samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);

        if max_abs > 0.0 && max_abs != 1.0 {
            let scale = 1.0 / max_abs;
            for sample in &mut self.samples {
                *sample *= scale;
            }
            info!("音频已标准化，缩放因子: {scale:.3}");
        }
    }

    /// 应用增益
    pub fn apply_gain(&mut self, gain_db: f32) {
        let gain_linear = 10.0f32.powf(gain_db / 20.0);
        for sample in &mut self.samples {
            *sample *= gain_linear;
            // 防止削波
            *sample = sample.clamp(-1.0, 1.0);
        }
        info!("应用增益: {gain_db:.1} dB (线性: {gain_linear:.3})");
    }

    /// 检查是否与Whisper兼容
    pub fn is_whisper_compatible(&self) -> bool {
        self.config.is_whisper_compatible()
    }
}

/// 从WAV文件读取音频数据
pub fn read_wav_file<P: AsRef<Path>>(path: P) -> SttResult<AudioData> {
    let path = path.as_ref();

    if !path.exists() {
        return Err(SttError::file_not_found(path.display().to_string()));
    }

    info!("读取WAV文件: {}", path.display());

    let mut reader = WavReader::open(path)
        .map_err(|e| SttError::AudioFileError(format!("打开WAV文件失败: {e}")))?;

    let spec = reader.spec();
    let config = AudioConfig {
        sample_rate: spec.sample_rate,
        channels: spec.channels,
        bit_depth: spec.bits_per_sample,
    };

    info!(
        "WAV文件信息: {}Hz, {}声道, {}位",
        config.sample_rate, config.channels, config.bit_depth
    );

    // 读取样本数据
    let samples: Vec<f32> = match spec.sample_format {
        SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| SttError::AudioFileError(format!("读取浮点样本失败: {e}")))?,
        SampleFormat::Int => {
            let int_samples: Result<Vec<i32>, _> = reader.samples::<i32>().collect();
            match int_samples {
                Ok(samples) => {
                    // 根据位深度转换为浮点数
                    let max_value = match spec.bits_per_sample {
                        16 => i16::MAX as f32,
                        24 => 8388607.0, // 2^23 - 1
                        32 => i32::MAX as f32,
                        _ => {
                            return Err(SttError::UnsupportedFormat(format!(
                                "不支持的位深度: {}",
                                spec.bits_per_sample
                            )));
                        }
                    };
                    samples.into_iter().map(|x| x as f32 / max_value).collect()
                }
                Err(e) => return Err(SttError::AudioFileError(format!("读取整数样本失败: {e}"))),
            }
        }
    };

    info!("成功读取 {} 个样本", samples.len());
    Ok(AudioData::new(samples, config))
}

/// 将音频数据写入WAV文件
pub fn write_wav_file<P: AsRef<Path>>(audio: &AudioData, path: P) -> SttResult<()> {
    let path = path.as_ref();

    info!("写入WAV文件: {}", path.display());

    let spec = WavSpec {
        channels: audio.config.channels,
        sample_rate: audio.config.sample_rate,
        bits_per_sample: audio.config.bit_depth,
        sample_format: if audio.config.bit_depth == 32 {
            SampleFormat::Float
        } else {
            SampleFormat::Int
        },
    };

    let mut writer = WavWriter::create(path, spec)
        .map_err(|e| SttError::AudioFileError(format!("创建WAV文件失败: {e}")))?;

    // 写入样本数据
    match spec.sample_format {
        SampleFormat::Float => {
            for &sample in &audio.samples {
                writer
                    .write_sample(sample)
                    .map_err(|e| SttError::AudioFileError(format!("写入浮点样本失败: {e}")))?;
            }
        }
        SampleFormat::Int => {
            let max_value = match audio.config.bit_depth {
                16 => i16::MAX as f32,
                24 => 8388607.0,
                32 => i32::MAX as f32,
                _ => {
                    return Err(SttError::UnsupportedFormat(format!(
                        "不支持的位深度: {}",
                        audio.config.bit_depth
                    )));
                }
            };

            for &sample in &audio.samples {
                let int_sample = (sample * max_value).round() as i32;
                writer
                    .write_sample(int_sample)
                    .map_err(|e| SttError::AudioFileError(format!("写入整数样本失败: {e}")))?;
            }
        }
    }

    writer
        .finalize()
        .map_err(|e| SttError::AudioFileError(format!("完成WAV文件写入失败: {e}")))?;

    info!("成功写入 {} 个样本到WAV文件", audio.samples.len());
    Ok(())
}

/// 检测音频文件格式
pub fn detect_audio_format<P: AsRef<Path>>(path: P) -> Option<AudioFormat> {
    let path = path.as_ref();

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        AudioFormat::from_extension(ext)
    } else {
        None
    }
}

/// 验证音频文件是否存在且可读
pub fn validate_audio_file<P: AsRef<Path>>(path: P) -> SttResult<()> {
    let path = path.as_ref();

    if !path.exists() {
        return Err(SttError::file_not_found(path.display().to_string()));
    }

    if !path.is_file() {
        return Err(SttError::AudioFileError(format!(
            "路径不是文件: {}",
            path.display()
        )));
    }

    // 检查文件扩展名
    if detect_audio_format(path).is_none() {
        warn!("无法识别音频文件格式: {}", path.display());
    }

    Ok(())
}

/// 转换i16样本到f32
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&x| x as f32 / i16::MAX as f32)
        .collect()
}

/// 转换f32样本到i16
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&x| (x.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect()
}

/// 计算音频的RMS（均方根）值
pub fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();

    (sum_squares / samples.len() as f32).sqrt()
}

/// 计算音频的峰值
pub fn calculate_peak(samples: &[f32]) -> f32 {
    samples.iter().map(|&x| x.abs()).fold(0.0f32, f32::max)
}

/// 检测音频中的静音段
pub fn detect_silence(
    samples: &[f32],
    threshold: f32,
    min_duration_ms: u32,
    sample_rate: u32,
) -> Vec<(usize, usize)> {
    let min_samples = (min_duration_ms as f32 * sample_rate as f32 / 1000.0) as usize;
    let mut silence_segments = Vec::new();
    let mut silence_start = None;

    for (i, &sample) in samples.iter().enumerate() {
        if sample.abs() < threshold {
            if silence_start.is_none() {
                silence_start = Some(i);
            }
        } else if let Some(start) = silence_start {
            if i - start >= min_samples {
                silence_segments.push((start, i));
            }
            silence_start = None;
        }
    }

    // 处理结尾的静音
    if let Some(start) = silence_start {
        if samples.len() - start >= min_samples {
            silence_segments.push((start, samples.len()));
        }
    }

    silence_segments
}

/// 移除音频开头和结尾的静音
pub fn trim_silence(audio: &AudioData, threshold: f32, min_duration_ms: u32) -> AudioData {
    let silence_segments = detect_silence(
        &audio.samples,
        threshold,
        min_duration_ms,
        audio.config.sample_rate,
    );

    if silence_segments.is_empty() {
        return audio.clone();
    }

    let channels = audio.config.channels as usize;

    // 找到开头的静音
    let start_trim = silence_segments
        .first()
        .filter(|(start, _)| *start == 0)
        .map(|(_, end)| *end)
        .unwrap_or(0);

    // 找到结尾的静音
    let end_trim = silence_segments
        .last()
        .filter(|(_, end)| *end == audio.samples.len())
        .map(|(start, _)| *start)
        .unwrap_or(audio.samples.len());

    if start_trim >= end_trim {
        // 整个音频都是静音
        warn!("整个音频文件都是静音");
        return AudioData::new(vec![0.0; channels], audio.config.clone());
    }

    // 提取非静音部分
    let trimmed_samples = audio.samples[start_trim * channels..end_trim * channels].to_vec();

    info!(
        "移除静音: 开头 {:.2}s, 结尾 {:.2}s",
        start_trim as f32 / audio.config.sample_rate as f32,
        (audio.samples.len() / channels - end_trim) as f32 / audio.config.sample_rate as f32
    );

    AudioData::new(trimmed_samples, audio.config.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_audio_format_unknown() {
        assert!(detect_audio_format("/tmp/noext").is_none());
        assert_eq!(detect_audio_format("a.wav"), Some(AudioFormat::Wav));
    }

    #[test]
    fn test_validate_audio_file_not_found() {
        let res = validate_audio_file("/tmp/__definitely_not_exist__.wav");
        assert!(res.is_err());
    }

    #[test]
    fn test_rms_and_peak() {
        let samples = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let rms = calculate_rms(&samples);
        let peak = calculate_peak(&samples);
        assert!(rms > 0.0);
        assert_eq!(peak, 1.0);
    }
}
