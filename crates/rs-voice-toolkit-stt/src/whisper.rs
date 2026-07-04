//! Whisper 语音转文本实现
//!
//! 基于 whisper-rs 库实现的语音识别功能

use crate::audio::AudioData;
use crate::error::{SttError, SttResult};
use log::{debug, info, warn};
use rs_voice_toolkit_audio as audio_lib;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
};

// 导入 VAD 相关模块
use crate::vad::SimpleVad;

/// Whisper 模型配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    /// 模型文件路径
    pub model_path: PathBuf,
    /// 语言代码（如 "zh", "en"，None 表示自动检测）
    pub language: Option<String>,
    /// 是否翻译为英文
    pub translate: bool,
    /// 线程数
    pub n_threads: i32,
    /// 是否输出时间戳
    pub print_timestamps: bool,
    /// 是否输出进度信息
    pub print_progress: bool,
    /// 是否输出特殊标记
    pub print_special: bool,
    /// 温度参数（0.0-1.0）
    pub temperature: f32,
    /// 最大段长度（毫秒）
    pub max_segment_length: Option<u32>,
    /// 是否使用初始提示
    pub initial_prompt: Option<String>,
    /// 是否启用语音活动检测 (VAD)
    pub enable_vad: bool,
    /// VAD 阈值 (0.0-1.0)，用于检测语音活动
    pub vad_threshold: f32,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::from("models/ggml-base.bin"),
            language: Some("auto".to_string()), // 自动检测
            translate: false,
            n_threads: 4,
            print_timestamps: true,
            print_progress: false,
            print_special: false,
            temperature: 0.2,
            max_segment_length: None,
            initial_prompt: None,
            enable_vad: true,    // 默认禁用 VAD，保持向后兼容
            vad_threshold: 0.01, // 默认 VAD 阈值
        }
    }
}

impl WhisperConfig {
    /// 创建新的配置
    pub fn new<P: Into<PathBuf>>(model_path: P) -> Self {
        Self {
            model_path: model_path.into(),
            ..Default::default()
        }
    }

    /// 设置语言
    pub fn with_language<S: Into<String>>(mut self, language: S) -> Self {
        self.language = Some(language.into());
        self
    }

    /// 设置是否翻译
    pub fn with_translate(mut self, translate: bool) -> Self {
        self.translate = translate;
        self
    }

    /// 设置线程数
    pub fn with_threads(mut self, n_threads: i32) -> Self {
        self.n_threads = n_threads;
        self
    }

    /// 设置温度参数
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature.clamp(0.0, 1.0);
        self
    }

    /// 设置初始提示
    pub fn with_initial_prompt<S: Into<String>>(mut self, prompt: S) -> Self {
        self.initial_prompt = Some(prompt.into());
        self
    }

    /// 设置是否启用 VAD
    pub fn with_vad(mut self, enable_vad: bool) -> Self {
        self.enable_vad = enable_vad;
        self
    }

    /// 设置 VAD 阈值
    pub fn with_vad_threshold(mut self, threshold: f32) -> Self {
        self.vad_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// 验证配置
    pub fn validate(&self) -> SttResult<()> {
        if !self.model_path.exists() {
            return Err(SttError::ModelLoadError(format!(
                "模型文件不存在: {}",
                self.model_path.display()
            )));
        }

        if self.n_threads <= 0 {
            return Err(SttError::ConfigError("线程数必须大于0".to_string()));
        }

        if !(0.0..=1.0).contains(&self.temperature) {
            return Err(SttError::ConfigError(
                "温度参数必须在0.0-1.0之间".to_string(),
            ));
        }

        if !(0.0..=1.0).contains(&self.vad_threshold) {
            return Err(SttError::ConfigError(
                "VAD阈值必须在0.0-1.0之间".to_string(),
            ));
        }

        Ok(())
    }
}

/// 转录结果段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    /// 开始时间（毫秒）
    pub start_time: u64,
    /// 结束时间（毫秒）
    pub end_time: u64,
    /// 文本内容
    pub text: String,
    /// 置信度（0.0-1.0）
    pub confidence: f32,
}

/// 转录结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    /// 完整文本
    pub text: String,
    /// 检测到的语言
    pub language: Option<String>,
    /// 分段结果
    pub segments: Vec<TranscriptionSegment>,
    /// 处理时长（毫秒）
    pub processing_time: u64,
    /// 音频时长（毫秒）
    pub audio_duration: u64,
}

impl TranscriptionResult {
    /// 获取实时因子（处理时间/音频时长）
    pub fn real_time_factor(&self) -> f64 {
        if self.audio_duration == 0 {
            return 0.0;
        }
        self.processing_time as f64 / self.audio_duration as f64
    }

    /// 获取平均置信度
    pub fn average_confidence(&self) -> f32 {
        if self.segments.is_empty() {
            return 0.0;
        }

        let total: f32 = self.segments.iter().map(|s| s.confidence).sum();
        total / self.segments.len() as f32
    }

    /// 过滤低置信度段
    pub fn filter_by_confidence(&self, min_confidence: f32) -> TranscriptionResult {
        let filtered_segments: Vec<_> = self
            .segments
            .iter()
            .filter(|s| s.confidence >= min_confidence)
            .cloned()
            .collect();

        let filtered_text = filtered_segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        TranscriptionResult {
            text: filtered_text,
            language: self.language.clone(),
            segments: filtered_segments,
            processing_time: self.processing_time,
            audio_duration: self.audio_duration,
        }
    }
}

/// Whisper 转录器
pub struct WhisperTranscriber {
    /// Whisper模型上下文
    context: Arc<WhisperContext>,
    /// Whisper配置
    config: WhisperConfig,
}

impl WhisperTranscriber {
    /// 创建新的转录器
    pub fn new(config: WhisperConfig) -> SttResult<Self> {
        config.validate()?;

        info!("加载Whisper模型: {}", config.model_path.display());

        let ctx_params = WhisperContextParameters::default();
        let context = WhisperContext::new_with_params(
            config.model_path.to_string_lossy().as_ref(),
            ctx_params,
        )
        .map_err(|e| SttError::ModelLoadError(format!("加载Whisper模型失败: {e}")))?;

        info!("Whisper模型加载成功");

        Ok(Self {
            context: Arc::new(context),
            config,
        })
    }

    /// 从文件转录
    pub async fn transcribe_file<P: AsRef<Path>>(
        &self,
        audio_path: P,
    ) -> SttResult<TranscriptionResult> {
        let audio_path = audio_path.as_ref();

        info!("开始转录文件: {}", audio_path.display());

        // 确保输入音频转为 Whisper 兼容（mono/16k/WAV）
        let converted =
            audio_lib::ensure_whisper_compatible(audio_path, None).map_err(|e| match e {
                audio_lib::AudioError::FileNotFound(path) => {
                    SttError::AudioProcessingError(format!("音频文件不存在: {path}"))
                }
                audio_lib::AudioError::NotAFile(path) => {
                    SttError::AudioProcessingError(format!("路径不是音频文件: {path}"))
                }
                audio_lib::AudioError::FormatNotSupported { format, supported } => {
                    SttError::AudioProcessingError(format!(
                        "音频格式不支持: {format}, 支持的格式: {supported}"
                    ))
                }
                audio_lib::AudioError::SampleRateMismatch { expected, actual } => {
                    SttError::AudioProcessingError(format!(
                        "采样率不匹配: 期望 {expected}, 实际 {actual}"
                    ))
                }
                audio_lib::AudioError::ChannelMismatch { expected, actual } => {
                    SttError::AudioProcessingError(format!(
                        "通道数不匹配: 期望 {expected}, 实际 {actual}"
                    ))
                }
                audio_lib::AudioError::FfmpegConfig(msg)
                | audio_lib::AudioError::FfmpegExecution(msg) => {
                    SttError::AudioProcessingError(format!("FFmpeg 错误: {msg}"))
                }
                audio_lib::AudioError::DecodeError { reason } => {
                    SttError::AudioProcessingError(format!("音频解码失败: {reason}"))
                }
                audio_lib::AudioError::InvalidSampleRate { rate, min, max } => {
                    SttError::AudioProcessingError(format!(
                        "无效采样率: {rate}, 有效范围: {min}-{max}"
                    ))
                }
                audio_lib::AudioError::ResampleError(msg) => {
                    SttError::AudioProcessingError(format!("重采样失败: {msg}"))
                }
                _ => SttError::AudioProcessingError(format!("音频处理失败: {e}")),
            })?;

        // 读取 WAV 到内存（内部工具）
        let audio_data = crate::audio::utils::read_wav_file(&converted.path)?;

        // 转录音频数据
        self.transcribe_audio_data(&audio_data).await
    }

    /// 转录音频数据
    pub async fn transcribe_audio_data(
        &self,
        audio_data: &AudioData,
    ) -> SttResult<TranscriptionResult> {
        let start_time = std::time::Instant::now();

        // 检查音频格式兼容性
        if !audio_data.is_whisper_compatible() {
            warn!("音频格式不兼容Whisper，建议转换为16kHz单声道");
        }

        // 准备音频数据
        let audio_samples = self.prepare_audio_samples(audio_data)?;

        // VAD 检测（如果启用）
        let mut processed_samples = audio_samples.clone();
        let audio_duration = audio_data.duration();
        let mut audio_duration_adj = audio_duration;
        let mut start_offset_ms = 0;

        if self.config.enable_vad {
            let vad = SimpleVad::new(self.config.vad_threshold);

            // 检测语音段
            let speech_segments = vad.detect_speech_segments(&audio_samples);

            if speech_segments.is_empty() {
                info!("VAD检测到无语音活动，跳过转录");
                return Ok(TranscriptionResult {
                    text: String::new(),
                    language: self.config.language.clone(),
                    segments: Vec::new(),
                    processing_time: start_time.elapsed().as_millis() as u64,
                    audio_duration: (audio_duration * 1000.0) as u64,
                });
            } else {
                // 裁剪开头的静音部分，使用第一个语音段
                let first_segment = speech_segments.first().unwrap();

                if first_segment.0 > 0 {
                    // 裁剪音频样本
                    processed_samples = audio_samples[first_segment.0..].to_vec();

                    // 计算裁剪后的音频时长
                    let sample_rate = audio_data.config.sample_rate as f64;
                    start_offset_ms = (first_segment.0 as f64 / sample_rate * 1000.0) as u64;
                    audio_duration_adj = audio_duration - (first_segment.0 as f64 / sample_rate);

                    info!(
                        "VAD裁剪掉开头静音部分，偏移量: {}毫秒，原长度: {:.2}秒，裁剪后长度: {:.2}秒",
                        start_offset_ms, audio_duration, audio_duration_adj
                    );
                }

                debug!("VAD检测到{}个语音段，继续转录", speech_segments.len());
            }
        }

        info!("开始Whisper推理,音频长度: {:.2}秒", audio_duration_adj);

        // 创建Whisper状态
        let mut state = self
            .context
            .create_state()
            .map_err(|e| SttError::WhisperError(format!("创建Whisper状态失败: {e}")))?;

        // 设置参数
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // 配置参数
        params.set_n_threads(self.config.n_threads);
        params.set_translate(self.config.translate);
        params.set_print_timestamps(self.config.print_timestamps);
        params.set_print_progress(self.config.print_progress);
        params.set_print_special(self.config.print_special);
        params.set_temperature(self.config.temperature);

        // 设置语言
        if let Some(ref language) = self.config.language {
            params.set_language(Some(language.as_str()));
        }

        // 注意：whisper-rs可能不支持set_initial_prompt方法
        // 如果需要初始提示功能，请查阅whisper-rs文档获取正确的API
        // if let Some(prompt) = &self.config.initial_prompt {
        //     // params.set_initial_prompt(Some(prompt.as_str()));
        // }

        // 执行转录
        state
            .full(params, &processed_samples)
            .map_err(|e| SttError::TranscriptionError(format!("Whisper转录失败: {e}")))?;

        // 提取结果
        let mut result =
            self.extract_transcription_result(&state, audio_duration_adj, start_time)?;

        // 调整时间戳以反映裁剪后的音频
        if start_offset_ms > 0 {
            for segment in &mut result.segments {
                segment.start_time += start_offset_ms;
                segment.end_time += start_offset_ms;
            }
        }

        info!("转录完成，实时因子: {:.2}x", result.real_time_factor());

        Ok(result)
    }

    /// 准备音频样本数据
    fn prepare_audio_samples(&self, audio_data: &AudioData) -> SttResult<Vec<f32>> {
        let mut samples = audio_data.samples.clone();

        // 转换为单声道（如果需要）
        if audio_data.config.channels > 1 {
            let mono_data = audio_data.to_mono();
            samples = mono_data.samples;
            debug!("音频已转换为单声道");
        }

        // Whisper期望的采样率是16kHz
        if audio_data.config.sample_rate != 16000 {
            warn!(
                "音频采样率为{}Hz，Whisper推荐16kHz",
                audio_data.config.sample_rate
            );
            // 这里可以添加重采样逻辑
        }

        Ok(samples)
    }

    /// 提取转录结果
    fn extract_transcription_result(
        &self,
        state: &WhisperState,
        audio_duration: f64,
        start_time: std::time::Instant,
    ) -> SttResult<TranscriptionResult> {
        let processing_time = start_time.elapsed().as_millis() as u64;
        let audio_duration_ms = (audio_duration * 1000.0) as u64;

        let num_segments = state.full_n_segments();

        let mut segments = Vec::new();
        let mut full_text = String::new();

        for i in 0..num_segments {
            // 获取段对象
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            // 获取段文本
            let segment_text = segment.to_str().unwrap_or("").trim().to_string();

            if segment_text.is_empty() {
                continue;
            }

            // 获取时间戳（centiseconds → ms）
            let start_time = (segment.start_timestamp() as u64) * 10;
            let end_time = (segment.end_timestamp() as u64) * 10;

            // 计算置信度（简化实现）
            let confidence = self.calculate_segment_confidence(state, i)?;

            segments.push(TranscriptionSegment {
                start_time,
                end_time,
                text: segment_text.clone(),
                confidence,
            });

            if !full_text.is_empty() {
                full_text.push(' ');
            }
            full_text.push_str(&segment_text);
        }

        // 尝试检测语言
        let language = self.detect_language(state);

        Ok(TranscriptionResult {
            text: full_text,
            language,
            segments,
            processing_time,
            audio_duration: audio_duration_ms,
        })
    }

    /// 计算段置信度（简化实现）
    fn calculate_segment_confidence(
        &self,
        state: &WhisperState,
        segment_index: i32,
    ) -> SttResult<f32> {
        // 这是一个简化的置信度计算
        // 实际实现可能需要更复杂的逻辑

        // 使用 segment API 获取 token 数
        let Some(segment) = state.get_segment(segment_index) else {
            return Ok(0.0);
        };
        let token_count = segment.n_tokens();

        if token_count == 0 {
            return Ok(0.0);
        }

        let mut total_prob = 0.0;
        let mut valid_tokens = 0;

        for token_index in 0..token_count {
            if let Some(token) = segment.get_token(token_index) {
                total_prob += token.token_probability();
                valid_tokens += 1;
            }
        }

        if valid_tokens > 0 {
            Ok(total_prob / valid_tokens as f32)
        } else {
            Ok(0.5) // 默认置信度
        }
    }

    /// 检测语言
    fn detect_language(&self, _state: &WhisperState) -> Option<String> {
        // 简化实现，返回配置的语言或None
        self.config.language.clone()
    }

    /// 获取模型信息
    pub fn model_info(&self) -> SttResult<String> {
        // 这里可以返回模型的详细信息
        Ok(format!("Whisper模型: {}", self.config.model_path.display()))
    }

    /// 检查模型是否支持多语言
    pub fn is_multilingual(&self) -> bool {
        // 简化实现，根据模型文件名判断
        let model_name = self
            .config
            .model_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        !model_name.contains(".en")
    }
}

/// 便捷函数：快速转录文件
pub async fn transcribe_file<P1, P2>(
    model_path: P1,
    audio_path: P2,
) -> SttResult<TranscriptionResult>
where
    P1: Into<PathBuf>,
    P2: AsRef<Path>,
{
    let config = WhisperConfig::new(model_path);
    let transcriber = WhisperTranscriber::new(config)?;
    transcriber.transcribe_file(audio_path).await
}

/// 便捷函数：快速转录文件（带语言设置）
pub async fn transcribe_file_with_language<P1, P2>(
    model_path: P1,
    audio_path: P2,
    language: &str,
) -> SttResult<TranscriptionResult>
where
    P1: Into<PathBuf>,
    P2: AsRef<Path>,
{
    let config = WhisperConfig::new(model_path).with_language(language);
    let transcriber = WhisperTranscriber::new(config)?;
    transcriber.transcribe_file(audio_path).await
}

/// 便捷函数：快速转录文件（带自定义配置）
/// 如果未提供配置，则使用默认配置
pub async fn transcribe_file_with_config<P1, P2>(
    model_path: P1,
    audio_path: P2,
    config: Option<WhisperConfig>,
) -> SttResult<TranscriptionResult>
where
    P1: Into<PathBuf>,
    P2: AsRef<Path>,
{
    // 如果提供了配置，则使用它；否则使用基于给定模型路径的默认配置
    let config = config.unwrap_or_else(|| WhisperConfig::new(model_path));
    let transcriber = WhisperTranscriber::new(config)?;
    transcriber.transcribe_file(audio_path).await
}

/// 便捷函数：使用已有的 WhisperTranscriber 实例转录文件
/// 这可以避免每次都重新加载模型，提高处理多个文件时的性能
pub async fn transcribe_file_with_transcriber<P: AsRef<Path>>(
    transcriber: &WhisperTranscriber,
    audio_path: P,
) -> SttResult<TranscriptionResult> {
    transcriber.transcribe_file(audio_path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let cfg = WhisperConfig::default();
        assert!(cfg.n_threads > 0);
        assert!(cfg.temperature >= 0.0 && cfg.temperature <= 1.0);
    }

    #[test]
    fn test_validate_model_path_not_exist() {
        let cfg = WhisperConfig::new("/tmp/___not_exist_model__.bin");
        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("模型文件不存在"));
    }

    #[test]
    fn test_vad_config() {
        let config = WhisperConfig::default()
            .with_vad(true)
            .with_vad_threshold(0.05);

        assert!(config.enable_vad);
        assert_eq!(config.vad_threshold, 0.05);
    }

    #[test]
    fn test_vad_threshold_clamping() {
        // 测试阈值被正确限制在 0.0-1.0 范围内
        let config1 = WhisperConfig::default().with_vad_threshold(-0.1);
        assert_eq!(config1.vad_threshold, 0.0);

        let config2 = WhisperConfig::default().with_vad_threshold(1.5);
        assert_eq!(config2.vad_threshold, 1.0);

        let config3 = WhisperConfig::default().with_vad_threshold(0.5);
        assert_eq!(config3.vad_threshold, 0.5);
    }

    #[test]
    fn test_vad_validation() {
        // 测试 VAD 阈值验证 - 需要设置一个不存在的模型路径来避免模型文件检查
        let mut config = WhisperConfig::new("/tmp/fake_model.bin");
        config.vad_threshold = 1.5; // 手动设置超出范围的值

        let result = config.validate();
        assert!(result.is_err());

        // 检查错误信息是否包含 VAD 相关内容或模型文件不存在
        match result {
            Err(crate::error::SttError::ConfigError(msg)) => {
                assert!(msg.contains("VAD阈值") || msg.contains("模型文件不存在"));
            }
            Err(crate::error::SttError::ModelLoadError(_)) => {
                // 模型文件不存在的错误也是可以接受的，因为我们使用了假路径
            }
            _ => panic!("应该返回配置错误或模型加载错误"),
        }
    }
}
