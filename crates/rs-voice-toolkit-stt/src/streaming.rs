//! 实时语音转录模块
//!
//! 提供实时音频流处理和转录功能，支持：
//! - 音频流缓冲和分段
//! - 实时转录处理
//! - VAD（语音活动检测）
//! - 流式结果输出

use crate::{
    audio::{AudioConfig, AudioData},
    error::{SttError, SttResult},
    vad::SimpleVad,
    whisper::{TranscriptionResult, WhisperConfig, WhisperTranscriber},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, mpsc::error::TryRecvError},
    time::sleep,
};

/// 流式转录配置
#[derive(Debug, Clone)]
pub struct StreamingConfig {
    /// 音频缓冲区大小（秒）
    pub buffer_duration: Duration,
    /// 转录间隔（秒）
    pub transcription_interval: Duration,
    /// 最小音频长度（秒）
    pub min_audio_length: Duration,
    /// 最大音频长度（秒）
    pub max_audio_length: Duration,
    /// 是否启用VAD
    pub enable_vad: bool,
    /// VAD阈值
    pub vad_threshold: f32,
    /// 静音超时（秒）
    pub silence_timeout: Duration,
    /// LocalAgreement 窗口大小 n（至少 2）
    pub local_agreement_n: usize,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            buffer_duration: Duration::from_secs(5),
            transcription_interval: Duration::from_millis(500),
            min_audio_length: Duration::from_millis(500),
            max_audio_length: Duration::from_secs(30),
            enable_vad: true,
            vad_threshold: 0.005,
            silence_timeout: Duration::from_secs(2),
            local_agreement_n: 3,
        }
    }
}

/// 流式转录事件
#[derive(Debug, Clone)]
pub enum StreamingEvent {
    /// 转录结果
    Transcription(TranscriptionResult),
    /// 语音开始
    SpeechStart,
    /// 语音结束
    SpeechEnd,
    /// 静音检测
    Silence,
    /// 错误事件
    Error(String),
}

/// 音频缓冲区
#[derive(Debug)]
struct AudioBuffer {
    /// 音频样本数据队列
    samples: VecDeque<f32>,
    /// 音频配置信息
    config: AudioConfig,
    /// 最大样本数量
    max_samples: usize,
}

impl AudioBuffer {
    fn new(config: AudioConfig, max_duration: Duration) -> Self {
        let max_samples = (config.sample_rate as f64 * max_duration.as_secs_f64()) as usize;
        Self {
            samples: VecDeque::with_capacity(max_samples),
            config,
            max_samples,
        }
    }

    fn push_samples(&mut self, new_samples: &[f32]) {
        for &sample in new_samples {
            if self.samples.len() >= self.max_samples {
                self.samples.pop_front();
            }
            self.samples.push_back(sample);
        }
    }

    #[allow(dead_code)]
    fn get_samples(&self, duration: Duration) -> Vec<f32> {
        let num_samples = (self.config.sample_rate as f64 * duration.as_secs_f64()) as usize;
        let start_idx = self.samples.len().saturating_sub(num_samples);
        self.samples.range(start_idx..).copied().collect()
    }

    #[allow(dead_code)]
    fn get_all_samples(&self) -> Vec<f32> {
        self.samples.iter().copied().collect()
    }

    fn duration(&self) -> Duration {
        let seconds = self.samples.len() as f64 / self.config.sample_rate as f64;
        Duration::from_secs_f64(seconds)
    }

    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    fn clear(&mut self) {
        self.samples.clear();
    }
}

// SimpleVad 现在从 vad 模块导入

/// 转录结果聚合器：基于 LocalAgreement-n 的前缀一致性确认
#[derive(Debug, Default)]
struct StreamingAggregator {
    /// 最近的转录文本队列
    last_texts: VecDeque<String>,
    /// 已确认的文本前缀
    confirmed_prefix: String,
    /// LocalAgreement 窗口大小
    n: usize,
}

impl StreamingAggregator {
    fn new(n: usize) -> Self {
        let n = n.max(2);
        Self {
            last_texts: VecDeque::with_capacity(n),
            confirmed_prefix: String::new(),
            n,
        }
    }

    fn push_and_confirm(&mut self, latest: &str) -> Option<String> {
        self.last_texts.push_back(latest.to_string());
        if self.last_texts.len() > self.n {
            self.last_texts.pop_front();
        }
        if self.last_texts.len() < self.n {
            return None;
        }

        let lcp = Self::longest_common_prefix(self.last_texts.iter().map(|s| s.as_str()));
        let lcp_trimmed = lcp.trim();
        if lcp_trimmed.len() > self.confirmed_prefix.len() {
            let addition = &lcp_trimmed[self.confirmed_prefix.len()..];
            self.confirmed_prefix = lcp_trimmed.to_string();
            return if addition.is_empty() {
                None
            } else {
                Some(addition.to_string())
            };
        }
        None
    }

    fn longest_common_prefix<'a, I: Iterator<Item = &'a str>>(mut it: I) -> String {
        if let Some(first) = it.next() {
            let mut prefix = first.as_bytes().to_vec();
            for s in it {
                let bytes = s.as_bytes();
                let mut i = 0;
                while i < prefix.len() && i < bytes.len() && prefix[i] == bytes[i] {
                    i += 1;
                }
                prefix.truncate(i);
                if prefix.is_empty() {
                    break;
                }
            }
            return String::from_utf8(prefix).unwrap_or_default();
        }
        String::new()
    }
}

/// 实时语音转录器
pub struct StreamingTranscriber {
    /// Whisper转录器实例
    transcriber: Arc<WhisperTranscriber>,
    /// 流式转录配置
    config: StreamingConfig,
    /// 音频配置
    audio_config: AudioConfig,
    /// 音频缓冲区（线程安全）
    buffer: Arc<Mutex<AudioBuffer>>,
    /// 语音活动检测器（可选）
    vad: Option<SimpleVad>,
    /// 事件发送器（用于输出转录结果和状态）
    event_sender: Option<mpsc::UnboundedSender<StreamingEvent>>,
    /// 运行状态标志（线程安全）
    is_running: Arc<Mutex<bool>>,
    /// 音频数据发送器
    audio_sender: Option<mpsc::UnboundedSender<Vec<f32>>>,
    /// 音频处理任务句柄
    audio_task_handle: Option<tokio::task::JoinHandle<()>>,
    /// 转录处理任务句柄
    transcription_task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl StreamingTranscriber {
    /// 创建新的流式转录器
    pub fn new(
        whisper_config: WhisperConfig,
        streaming_config: StreamingConfig,
        audio_config: AudioConfig,
    ) -> SttResult<Self> {
        let transcriber = Arc::new(WhisperTranscriber::new(whisper_config)?);
        let buffer = Arc::new(Mutex::new(AudioBuffer::new(
            audio_config.clone(),
            streaming_config.buffer_duration,
        )));

        let vad = if streaming_config.enable_vad {
            Some(SimpleVad::new_with_sample_rate(
                streaming_config.vad_threshold,
                audio_config.sample_rate,
            ))
        } else {
            None
        };

        Ok(Self {
            transcriber,
            config: streaming_config,
            audio_config,
            buffer,
            vad,
            event_sender: None,
            is_running: Arc::new(Mutex::new(false)),
            audio_sender: None,
            audio_task_handle: None,
            transcription_task_handle: None,
        })
    }

    /// 动态启用/禁用VAD
    pub fn set_vad_enabled(&mut self, enabled: bool) {
        if enabled && self.vad.is_none() {
            self.vad = Some(SimpleVad::new_with_sample_rate(
                self.config.vad_threshold,
                self.audio_config.sample_rate,
            ));
        } else if !enabled {
            self.vad = None;
        }
        self.config.enable_vad = enabled;
    }

    /// 设置VAD阈值
    pub fn set_vad_threshold(&mut self, threshold: f32) {
        self.config.vad_threshold = threshold;
        if let Some(ref mut vad) = self.vad {
            *vad = SimpleVad::new_with_sample_rate(threshold, self.audio_config.sample_rate);
        }
    }

    /// 获取当前VAD状态
    pub fn is_vad_enabled(&self) -> bool {
        self.config.enable_vad && self.vad.is_some()
    }

    /// 开始流式转录
    pub async fn start_streaming(&mut self) -> SttResult<mpsc::UnboundedReceiver<StreamingEvent>> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.event_sender = Some(tx.clone());
        let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<f32>>();
        self.audio_sender = Some(audio_tx);

        *self.is_running.lock().unwrap() = true;

        // 启动音频处理任务（异步解耦）
        let buffer = Arc::clone(&self.buffer);
        let is_running_audio = Arc::clone(&self.is_running);
        self.audio_task_handle = Some(tokio::spawn(async move {
            while *is_running_audio.lock().unwrap() {
                // 批量处理音频数据，减少锁竞争
                let mut batch = Vec::new();
                let mut batch_size = 0;
                const MAX_BATCH_SIZE: usize = 8192; // 约0.17秒@48kHz

                // 收集一批音频数据
                loop {
                    match audio_rx.try_recv() {
                        Ok(chunk) => {
                            batch_size += chunk.len();
                            batch.extend_from_slice(&chunk);
                            if batch_size >= MAX_BATCH_SIZE {
                                break;
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }

                // 批量写入缓冲区
                if !batch.is_empty() {
                    buffer.lock().unwrap().push_samples(&batch);
                }

                // 短暂休眠避免CPU占用过高
                sleep(Duration::from_millis(10)).await;
            }
        }));

        // 启动转录任务
        let transcriber = Arc::clone(&self.transcriber);
        let buffer_clone = Arc::clone(&self.buffer);
        let config = self.config.clone();
        let audio_config = self.audio_config.clone();
        let is_running = Arc::clone(&self.is_running);
        let vad = self.vad.clone();

        self.transcription_task_handle = Some(tokio::spawn(async move {
            let mut last_transcription = Instant::now();
            let mut speech_detected = false;
            let mut last_speech_time = Instant::now();
            let mut aggregator = StreamingAggregator::new(config.local_agreement_n);

            while *is_running.lock().unwrap() {
                sleep(Duration::from_millis(50)).await; // 更频繁的检查

                let now = Instant::now();
                let should_transcribe =
                    now.duration_since(last_transcription) >= config.transcription_interval;

                if should_transcribe {
                    let samples = {
                        let buffer_guard = buffer_clone.lock().unwrap();
                        if buffer_guard.duration() < config.min_audio_length {
                            continue;
                        }
                        buffer_guard.get_all_samples()
                    };

                    if samples.is_empty() {
                        continue;
                    }

                    // VAD检测
                    if let Some(ref vad_detector) = vad {
                        let has_speech = vad_detector.detect_speech(&samples);

                        if has_speech {
                            if !speech_detected {
                                let _ = tx.send(StreamingEvent::SpeechStart);
                                speech_detected = true;
                            }
                            last_speech_time = now;
                        } else if speech_detected {
                            // 检查静音超时
                            if now.duration_since(last_speech_time) >= config.silence_timeout {
                                let _ = tx.send(StreamingEvent::SpeechEnd);
                                let _ = tx.send(StreamingEvent::Silence);
                                speech_detected = false;

                                // 清空缓冲区
                                buffer_clone.lock().unwrap().clear();
                                continue;
                            }
                        }

                        // 如果没有检测到语音，跳过转录
                        if !has_speech && !speech_detected {
                            continue;
                        }
                    }

                    // 执行转录
                    let audio_data = AudioData::new(samples, audio_config.clone());
                    match transcriber.transcribe_audio_data(&audio_data).await {
                        Ok(result) => {
                            let text = result.text.trim().to_string();
                            if text.is_empty() {
                                // 跳过空结果
                            } else {
                                // 尝试确认文本
                                if let Some(confirmed_add) = aggregator.push_and_confirm(&text) {
                                    if !confirmed_add.trim().is_empty() {
                                        let confirmed = TranscriptionResult {
                                            text: confirmed_add,
                                            language: result.language.clone(),
                                            segments: Vec::new(),
                                            processing_time: result.processing_time,
                                            audio_duration: result.audio_duration,
                                        };
                                        let _ = tx.send(StreamingEvent::Transcription(confirmed));
                                    }
                                } else {
                                    // 对于单次转录，直接发送结果
                                    if config.local_agreement_n <= 1 {
                                        let direct_result = TranscriptionResult {
                                            text: text.clone(),
                                            language: result.language.clone(),
                                            segments: Vec::new(),
                                            processing_time: result.processing_time,
                                            audio_duration: result.audio_duration,
                                        };
                                        let _ =
                                            tx.send(StreamingEvent::Transcription(direct_result));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(StreamingEvent::Error(e.to_string()));
                        }
                    }

                    last_transcription = now;
                }
            }
        }));

        Ok(rx)
    }

    /// 停止流式转录
    pub fn stop_streaming(&mut self) {
        *self.is_running.lock().unwrap() = false;
        self.event_sender = None;
        self.audio_sender = None;

        // 取消任务（非阻塞）
        if let Some(handle) = self.audio_task_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.transcription_task_handle.take() {
            handle.abort();
        }
    }

    /// 异步停止流式转录并等待任务完成
    pub async fn stop_streaming_async(&mut self) {
        *self.is_running.lock().unwrap() = false;
        self.event_sender = None;
        self.audio_sender = None;

        // 等待任务完成
        if let Some(handle) = self.audio_task_handle.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.transcription_task_handle.take() {
            let _ = handle.await;
        }
    }

    /// 添加音频数据
    pub fn push_audio(&self, samples: &[f32]) -> SttResult<()> {
        if let Some(tx) = &self.audio_sender {
            let _ = tx.send(samples.to_vec());
            Ok(())
        } else {
            Err(SttError::other("转录器未运行"))
        }
    }

    /// 添加音频数据（i16格式）
    pub fn push_audio_i16(&self, samples: &[i16]) -> SttResult<()> {
        let f32_samples: Vec<f32> = samples.iter().map(|&x| x as f32 / 32768.0).collect();
        self.push_audio(&f32_samples)
    }

    /// 获取当前缓冲区状态
    pub fn buffer_info(&self) -> (Duration, usize) {
        let buffer_guard = self.buffer.lock().unwrap();
        (buffer_guard.duration(), buffer_guard.samples.len())
    }

    /// 检查是否正在运行
    pub fn is_running(&self) -> bool {
        *self.is_running.lock().unwrap()
    }

    /// 清空音频缓冲区
    pub fn clear_buffer(&self) {
        self.buffer.lock().unwrap().clear();
    }
}

/// 便捷函数：创建默认的流式转录器
pub fn create_streaming_transcriber(
    model_path: impl Into<std::path::PathBuf>,
) -> SttResult<StreamingTranscriber> {
    let whisper_config = WhisperConfig::new(model_path);
    let streaming_config = StreamingConfig::default();
    let audio_config = AudioConfig::whisper_optimized();

    StreamingTranscriber::new(whisper_config, streaming_config, audio_config)
}

/// 便捷函数：创建自定义配置的流式转录器
pub fn create_custom_streaming_transcriber(
    model_path: impl Into<std::path::PathBuf>,
    streaming_config: StreamingConfig,
    audio_config: AudioConfig,
) -> SttResult<StreamingTranscriber> {
    let whisper_config = WhisperConfig::new(model_path);
    StreamingTranscriber::new(whisper_config, streaming_config, audio_config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_audio_buffer() {
        let config = AudioConfig::whisper_optimized();
        let mut buffer = AudioBuffer::new(config.clone(), Duration::from_secs(1));

        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        buffer.push_samples(&samples);

        assert_eq!(buffer.samples.len(), 5);
        assert!(buffer.duration().as_secs_f64() > 0.0);

        let retrieved = buffer.get_all_samples();
        assert_eq!(retrieved, samples);
    }

    #[test]
    fn test_simple_vad() {
        let vad = SimpleVad::new_with_sample_rate(0.01, 16000);

        // 测试静音
        let silence = vec![0.0; 1000];
        assert!(!vad.detect_speech(&silence));

        // 测试有声音
        let speech = vec![0.1; 1000];
        assert!(vad.detect_speech(&speech));
    }

    #[test]
    fn test_streaming_config() {
        let config = StreamingConfig::default();
        assert_eq!(config.buffer_duration, Duration::from_secs(5));
        assert_eq!(config.transcription_interval, Duration::from_millis(500));
        assert!(config.enable_vad);
    }

    #[test]
    fn test_streaming_aggregator_lcp() {
        let mut agg = StreamingAggregator::new(3);

        // 测试最长公共前缀 - 完全相同的文本
        assert_eq!(agg.push_and_confirm("hello world"), None); // 第1个，不足3个
        assert_eq!(agg.push_and_confirm("hello world"), None); // 第2个，不足3个
        assert_eq!(
            agg.push_and_confirm("hello world"),
            Some("hello world".to_string())
        ); // 第3个，确认整个文本

        // 测试新的不同文本 - 需要重新开始聚合
        let mut agg2 = StreamingAggregator::new(3);
        assert_eq!(agg2.push_and_confirm("hello world"), None);
        assert_eq!(agg2.push_and_confirm("hello world"), None);
        assert_eq!(
            agg2.push_and_confirm("hello world"),
            Some("hello world".to_string())
        );

        // 现在添加不同的文本，应该只确认公共前缀
        assert_eq!(agg2.push_and_confirm("hello there"), None); // LCP是"hello"，但已经确认了"hello world"，所以没有新增
        assert_eq!(agg2.push_and_confirm("hello there"), None);
        assert_eq!(agg2.push_and_confirm("hello there"), None); // 公共前缀"hello"已经被包含在之前确认的"hello world"中
    }

    #[test]
    fn test_streaming_transcriber_creation() {
        use crate::whisper::WhisperConfig;

        let whisper_config = WhisperConfig::default();
        let streaming_config = StreamingConfig::default();
        let audio_config = AudioConfig::whisper_optimized();

        let result = StreamingTranscriber::new(whisper_config, streaming_config, audio_config);

        // 由于没有实际的模型文件，这里只测试配置是否正确
        assert!(result.is_err()); // 预期会失败，因为没有模型文件
    }

    #[test]
    fn test_streaming_config_customization() {
        let config = StreamingConfig {
            enable_vad: false,
            local_agreement_n: 1,
            vad_threshold: 0.001,
            ..Default::default()
        };

        assert!(!config.enable_vad);
        assert_eq!(config.local_agreement_n, 1);
        assert_eq!(config.vad_threshold, 0.001);
    }
}
