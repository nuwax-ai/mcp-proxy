//! 音频重采样器
//!
//! 使用 rubato 库进行高质量音频重采样

use super::AudioConfig;
use crate::error::{SttError, SttResult};
use log::info;

use audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

/// 音频重采样器
pub struct AudioResampler {
    /// 源采样率
    source_rate: u32,
    /// 目标采样率
    target_rate: u32,
    /// 声道数
    channels: usize,
}

impl AudioResampler {
    /// 创建新的重采样器
    pub fn new(source_rate: u32, target_rate: u32, channels: usize) -> SttResult<Self> {
        if source_rate == 0 || target_rate == 0 {
            return Err(SttError::ConfigError(
                "Sample rate cannot be zero".to_string(),
            ));
        }

        if channels == 0 {
            return Err(SttError::ConfigError(
                "Channel count cannot be zero".to_string(),
            ));
        }

        Ok(Self {
            source_rate,
            target_rate,
            channels,
        })
    }

    /// 从音频配置创建重采样器
    pub fn from_configs(source: &AudioConfig, target: &AudioConfig) -> SttResult<Self> {
        Self::new(
            source.sample_rate,
            target.sample_rate,
            source.channels as usize,
        )
    }

    /// 检查是否需要重采样
    pub fn needs_resampling(&self) -> bool {
        self.source_rate != self.target_rate
    }

    /// 计算重采样比率
    pub fn ratio(&self) -> f64 {
        self.target_rate as f64 / self.source_rate as f64
    }

    /// 重采样音频数据
    pub fn resample(&self, input: &[f32]) -> SttResult<Vec<f32>> {
        if !self.needs_resampling() {
            info!("Sample rates are the same, no resampling needed");
            return Ok(input.to_vec());
        }

        info!(
            "Starting resampling: {} Hz -> {} Hz",
            self.source_rate, self.target_rate
        );

        // 计算输出长度
        let input_frames = input.len() / self.channels;
        let output_frames = (input_frames as f64 * self.ratio()).round() as usize;

        // 创建重采样器参数
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        // 创建重采样器
        let mut resampler = Async::<f32>::new_sinc(
            self.ratio(),
            2.0, // 最大相对误差
            &params,
            input_frames,
            self.channels,
            FixedAsync::Input,
        )
        .map_err(|e| SttError::ResamplingError(format!("Failed to create resampler: {e}")))?;

        // 将交错音频数据转换为分离的声道数据
        let mut input_channels = vec![Vec::with_capacity(input_frames); self.channels];
        for (i, &sample) in input.iter().enumerate() {
            let channel = i % self.channels;
            input_channels[channel].push(sample);
        }

        // 准备输入数据
        let input_adapter =
            SequentialSliceOfVecs::new(&input_channels, self.channels, input_frames).map_err(
                |e| SttError::ResamplingError(format!("Failed to prepare input data: {e}")),
            )?;

        // 执行重采样
        let output_data = resampler
            .process(&input_adapter, 0, None)
            .map_err(|e| SttError::ResamplingError(format!("Resampling failed: {e}")))?;

        // 提取输出数据
        let output_channels_data = output_data.take_data();

        // 将分离的声道数据转换回交错格式
        // InterleavedOwned 格式是交错的，直接使用
        let output = output_channels_data;

        info!("Resampling done: {input_frames} frames -> {output_frames} frames");
        Ok(output)
    }

    /// 重采样音频数据（i16格式）
    pub fn resample_i16(&self, input: &[i16]) -> SttResult<Vec<i16>> {
        // 转换为 f32
        let input_f32: Vec<f32> = input.iter().map(|&x| x as f32 / i16::MAX as f32).collect();

        // 重采样
        let output_f32 = self.resample(&input_f32)?;

        // 转换回 i16
        let output_i16: Vec<i16> = output_f32
            .iter()
            .map(|&x| (x * i16::MAX as f32).round() as i16)
            .collect();

        Ok(output_i16)
    }

    /// 批量重采样（流式处理）
    pub fn resample_streaming(&self, input_chunks: Vec<&[f32]>) -> SttResult<Vec<f32>> {
        if !self.needs_resampling() {
            // 如果不需要重采样，直接连接所有块
            let mut result = Vec::new();
            for chunk in input_chunks {
                result.extend_from_slice(chunk);
            }
            return Ok(result);
        }

        let mut all_output = Vec::new();

        for chunk in input_chunks {
            let resampled_chunk = self.resample(chunk)?;
            all_output.extend(resampled_chunk);
        }

        Ok(all_output)
    }
}

/// 重采样质量设置
#[derive(Debug, Clone, Copy)]
pub enum ResampleQuality {
    /// 快速重采样（低质量）
    Fast,
    /// 标准重采样（中等质量）
    Standard,
    /// 高质量重采样（慢速）
    High,
}

impl ResampleQuality {
    /// 获取对应的 sinc_len 参数
    pub fn sinc_len(&self) -> usize {
        match self {
            ResampleQuality::Fast => 64,
            ResampleQuality::Standard => 256,
            ResampleQuality::High => 1024,
        }
    }

    /// 获取对应的过采样因子
    pub fn oversampling_factor(&self) -> usize {
        match self {
            ResampleQuality::Fast => 64,
            ResampleQuality::Standard => 256,
            ResampleQuality::High => 512,
        }
    }
}

/// 高级重采样器，支持质量设置
pub struct AdvancedResampler {
    base: AudioResampler,
    quality: ResampleQuality,
}

impl AdvancedResampler {
    /// 创建高级重采样器
    pub fn new(
        source_rate: u32,
        target_rate: u32,
        channels: usize,
        quality: ResampleQuality,
    ) -> SttResult<Self> {
        let base = AudioResampler::new(source_rate, target_rate, channels)?;
        Ok(Self { base, quality })
    }

    /// 使用指定质量进行重采样
    pub fn resample_with_quality(&self, input: &[f32]) -> SttResult<Vec<f32>> {
        if !self.base.needs_resampling() {
            return Ok(input.to_vec());
        }

        info!(
            "Starting high-quality resampling: {} Hz -> {} Hz (quality: {:?})",
            self.base.source_rate, self.base.target_rate, self.quality
        );

        // 使用自定义参数创建重采样器
        let params = SincInterpolationParameters {
            sinc_len: self.quality.sinc_len(),
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: self.quality.oversampling_factor(),
            window: WindowFunction::BlackmanHarris2,
        };

        let input_frames = input.len() / self.base.channels;

        let mut resampler = Async::<f32>::new_sinc(
            self.base.ratio(),
            2.0,
            &params,
            input_frames,
            self.base.channels,
            FixedAsync::Input,
        )
        .map_err(|e| {
            SttError::ResamplingError(format!("Failed to create high-quality resampler: {e}"))
        })?;

        // 执行重采样（与基础版本相同的逻辑）
        let mut input_channels = vec![Vec::with_capacity(input_frames); self.base.channels];
        for (i, &sample) in input.iter().enumerate() {
            let channel = i % self.base.channels;
            input_channels[channel].push(sample);
        }

        // 准备输入数据
        let input_adapter =
            SequentialSliceOfVecs::new(&input_channels, self.base.channels, input_frames).map_err(
                |e| SttError::ResamplingError(format!("Failed to prepare input data: {e}")),
            )?;

        // 执行重采样
        let output_data = resampler.process(&input_adapter, 0, None).map_err(|e| {
            SttError::ResamplingError(format!("High-quality resampling failed: {e}"))
        })?;

        // 提取输出数据
        let output = output_data.take_data();

        Ok(output)
    }
}
