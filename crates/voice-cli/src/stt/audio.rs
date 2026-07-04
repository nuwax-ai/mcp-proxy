//! 音频预处理：任意音视频文件 → Whisper 输入 f32 samples（16kHz / mono / [-1.0, 1.0]）。
//!
//! 链路：ffmpeg-sidecar 转码到 16k/mono/s16le WAV（临时文件）→
//!      transcribe-rs `read_wav_samples` → `Vec<f32>`。
//!
//! 输入无音频流时 ffmpeg 报错 → `SttError::Audio`（HTTP 调用方归类 400 BadRequest）。

use std::path::Path;

use ffmpeg_sidecar::command::FfmpegCommand;

use crate::stt::error::SttError;

/// 把任意音频/视频文件转为 Whisper 所需的 f32 samples。
///
/// 输入可以是 mp3/m4a/wav/mp4/... 任意 ffmpeg 支持的格式；
/// 输出采样率固定 16kHz、单声道、f32 归一化到 [-1.0, 1.0]。
pub fn to_whisper_samples(input: &Path) -> Result<Vec<f32>, SttError> {
    if !input.exists() {
        return Err(SttError::Audio(format!(
            "输入文件不存在: {}",
            input.display()
        )));
    }

    // 临时 wav 文件（NamedTempFile 在 drop 时自动删除）
    let temp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .map_err(|e| SttError::Audio(format!("创建临时文件失败: {e}")))?;
    let temp_path = temp.path();

    run_ffmpeg_convert(input, temp_path)?;

    let samples = transcribe_rs::audio::read_wav_samples(temp_path)?;

    drop(temp); // 删除临时文件
    Ok(samples)
}

/// ffmpeg-sidecar 转 16k/mono/s16le WAV。
///
/// 用 `wait()` 而非 `iter()`：iter() 是为实时帧捕获（stdout pipe）设计的，
/// 文件→文件转换的 output 不是 stdout，iter() 会误报 "No streams found"。
fn run_ffmpeg_convert(input: &Path, output: &Path) -> Result<(), SttError> {
    let input_str = input.to_str().unwrap_or_default();
    let output_str = output.to_str().unwrap_or_default();

    let mut child = FfmpegCommand::new()
        .input(input_str)
        .args([
            "-filter:a",
            "aformat=sample_fmts=s16:channel_layouts=mono:sample_rates=16000",
        ])
        .overwrite() // -y：覆盖已存在的 temp 文件（NamedTempFile 创建时即存在，否则 ffmpeg 拒绝写入）
        .output(output_str)
        .spawn()
        .map_err(|e| SttError::Audio(format!("ffmpeg 启动失败: {e}")))?;

    let status = child
        .wait()
        .map_err(|e| SttError::Audio(format!("ffmpeg 等待失败: {e}")))?;
    if !status.success() {
        return Err(SttError::Audio(format!(
            "ffmpeg 转换失败，退出码: {status}"
        )));
    }
    Ok(())
}
