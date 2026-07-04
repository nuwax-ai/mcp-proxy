//! 音频格式转换器
//!
//! 使用 ez-ffmpeg 进行音频格式转换，将各种音频格式转换为 Whisper 兼容的格式

use super::{AudioConfig, AudioFormat};
use crate::error::{SttError, SttResult};
use log::{info, warn};
use std::path::{Path, PathBuf};

use ffmpeg_sidecar::command::FfmpegCommand;

/// 音频转换器
pub struct AudioConverter {
    /// 目标音频配置
    #[allow(dead_code)]
    target_config: AudioConfig,
    /// 临时文件目录
    temp_dir: Option<PathBuf>,
}

impl AudioConverter {
    /// 创建新的音频转换器
    pub fn new(target_config: AudioConfig) -> Self {
        Self {
            target_config,
            temp_dir: None,
        }
    }

    /// 创建默认的音频转换器（Whisper优化）
    pub fn whisper_optimized() -> Self {
        Self::new(AudioConfig::whisper_optimized())
    }

    /// 设置临时文件目录
    pub fn with_temp_dir<P: Into<PathBuf>>(mut self, temp_dir: P) -> Self {
        self.temp_dir = Some(temp_dir.into());
        self
    }

    /// 转换音频文件到目标格式
    pub async fn convert_to_wav<P: AsRef<Path>>(
        &self,
        input_path: P,
        output_path: Option<P>,
    ) -> SttResult<PathBuf> {
        let input = input_path.as_ref();

        // 检查输入文件是否存在
        if !input.exists() {
            return Err(SttError::file_not_found(input.display().to_string()));
        }

        // 确定输出路径
        let output = match output_path {
            Some(path) => path.as_ref().to_path_buf(),
            None => self.generate_output_path(input)?,
        };

        // 检查是否需要转换
        if let Some(format) = self.detect_format(input)? {
            if format.is_whisper_native() && self.is_config_compatible(input).await? {
                info!("文件已经是兼容格式，无需转换: {}", input.display());
                return Ok(input.to_path_buf());
            }
        }

        // 执行转换
        self.convert_with_ffmpeg(input, &output).await?;

        info!("音频转换完成: {} -> {}", input.display(), output.display());
        Ok(output)
    }

    /// 检测音频文件格式
    fn detect_format<P: AsRef<Path>>(&self, path: P) -> SttResult<Option<AudioFormat>> {
        let path = path.as_ref();

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            Ok(AudioFormat::from_extension(ext))
        } else {
            warn!("无法从文件扩展名检测音频格式: {}", path.display());
            Ok(None)
        }
    }

    /// 检查音频配置是否兼容
    async fn is_config_compatible<P: AsRef<Path>>(&self, _path: P) -> SttResult<bool> {
        // 这里应该检查音频文件的实际参数
        // 目前返回 false 以确保转换
        // 在实际实现中，可以使用 ffprobe 或其他工具检查音频参数
        warn!("音频配置兼容性检查未实现，默认进行转换");
        Ok(false)
    }

    /// 生成输出文件路径
    fn generate_output_path<P: AsRef<Path>>(&self, input_path: P) -> SttResult<PathBuf> {
        let input = input_path.as_ref();

        let output_dir = if let Some(temp_dir) = &self.temp_dir {
            temp_dir.clone()
        } else {
            input
                .parent()
                .ok_or_else(|| SttError::other("无法确定输出目录"))?
                .to_path_buf()
        };

        let file_stem = input
            .file_stem()
            .ok_or_else(|| SttError::other("无法获取文件名"))?
            .to_str()
            .ok_or_else(|| SttError::other("文件名包含无效字符"))?;

        let output_filename = format!("{file_stem}_converted.wav");
        Ok(output_dir.join(output_filename))
    }

    /// 使用 FFmpeg 进行音频转换
    async fn convert_with_ffmpeg<P: AsRef<Path>>(
        &self,
        input_path: P,
        output_path: P,
    ) -> SttResult<()> {
        let input = input_path.as_ref();
        let output = output_path.as_ref();

        info!("开始音频转换: {} -> {}", input.display(), output.display());

        // 使用 ffmpeg-sidecar 进行音频转换
        let filter = "aformat=sample_fmts=s16:channel_layouts=mono:sample_rates=16000";

        let status = FfmpegCommand::new()
            .input(input.to_string_lossy())
            .args(["-filter:a", filter])
            .overwrite()
            .output(output.to_string_lossy())
            .spawn()?
            .wait()?;

        if !status.success() {
            return Err(SttError::AudioProcessingError("FFmpeg转换失败".to_string()));
        }

        Ok(())
    }

    /// 批量转换音频文件
    pub async fn convert_batch(
        &self,
        input_files: &[PathBuf],
        output_dir: Option<&PathBuf>,
    ) -> SttResult<Vec<PathBuf>> {
        let mut results = Vec::new();

        for input_path in input_files {
            let output_path = if let Some(dir) = output_dir {
                let filename = input_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("converted");
                Some(dir.join(format!("{filename}.wav")))
            } else {
                None
            };

            match self.convert_to_wav(input_path, output_path.as_ref()).await {
                Ok(path) => results.push(path),
                Err(e) => {
                    log::warn!("转换文件失败 {input_path:?}: {e}");
                    continue;
                }
            }
        }

        Ok(results)
    }
}

impl Default for AudioConverter {
    fn default() -> Self {
        Self::whisper_optimized()
    }
}
