//! # STT (Speech-to-Text) Module - 语音转文本模块
//!
//! 这个模块提供了基于 OpenAI Whisper 模型的高质量语音识别功能，
//! 支持文件转录、实时流式处理、语音活动检测等多种功能。
//!
//! ## 主要功能
//!
//! ### 核心特性
//! - **高精度识别**: 基于 Whisper 模型，支持多种语言
//! - **文件转录**: 支持多种音频格式的批量转录
//! - **实时流式处理**: 支持音频流的实时转录
//! - **语音活动检测**: 智能检测语音片段，提高处理效率
//! - **多模型支持**: 支持 tiny、base、small、medium、large 等不同规模的模型
//! - **性能监控**: 提供详细的性能指标和基准测试
//!
//! ### 支持的音频格式
//! - **WAV**: 原生支持，无需转换
//! - **MP3**: 自动转换为兼容格式
//! - **FLAC**: 自动转换为兼容格式
//! - **M4A**: 自动转换为兼容格式
//! - **OGG**: 自动转换为兼容格式
//!
//! ## 快速开始
//!
//! ### 基本文件转录
//!
//! ```rust
//! use rs_voice_toolkit_stt::{transcribe_file, WhisperConfig, SttError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), SttError> {
//!     let model_path = "models/ggml-base.bin";
//!     let audio_path = "audio/hello.wav";
//!     
//!     // 基本转录
//!     let result = transcribe_file(model_path, audio_path).await?;
//!     println!("转录结果: {}", result.text);
//!     println!("处理时间: {:?}", result.processing_time);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 自定义配置转录
//!
//! ```rust
//! use rs_voice_toolkit_stt::{transcribe_file_with_config, WhisperConfig, SttError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), SttError> {
//!     let model_path = "models/ggml-base.bin";
//!     let audio_path = "audio/hello.wav";
//!     
//!     // 自定义配置
//!     let config = WhisperConfig::new(model_path)
//!         .with_language("zh")          // 指定中文
//!         .with_temperature(0.2)         // 降低温度
//!         .with_vad(true)               // 启用语音活动检测
//!         .with_translate(false);        // 禁用翻译
//!     
//!     let result = transcribe_file_with_config(model_path, audio_path, Some(config)).await?;
//!     println!("转录结果: {}", result.text);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 流式转录
//!
//! ```rust
//! use rs_voice_toolkit_stt::{StreamingTranscriber, StreamingConfig, SttError};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), SttError> {
//!     let model_path = "models/ggml-base.bin";
//!     
//!     // 创建流式转录器
//!     let mut transcriber = StreamingTranscriber::new(model_path).await?;
//!     
//!     // 配置参数
//!     transcriber.set_language("auto")?;
//!     transcriber.set_task("transcribe")?;
//!     transcriber.enable_vad(true)?;
//!     
//!     // 模拟音频流处理
//!     let audio_chunks: Vec<Vec<f32>> = vec/*[音频数据块]*/;
//!     
//!     for chunk in audio_chunks {
//!         let segments = transcriber.process_audio(&chunk).await?;
//!         for segment in segments {
//!             println!("[{}s-{}s] {}", segment.start_time, segment.end_time, segment.text);
//!         }
//!     }
//!     
//!     // 获取最终结果
//!     let final_result = transcriber.finalize().await?;
//!     println!("最终转录: {}", final_result.text);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## 模型选择指南
//!
//! | 模型 | 大小 | 速度 | 准确度 | 适用场景 |
//! |------|------|------|--------|----------|
//! | tiny | ~39MB | 极快 | 一般 | 快速测试、实时应用 |
//! | base | ~74MB | 快 | 良好 | 日常应用、平衡性能 |
//! | small | ~244MB | 中等 | 很好 | 高要求应用 |
//! | medium | ~769MB | 较慢 | 优秀 | 专业应用 |
//! | large | ~1550MB | 慢 | 最佳 | 最高精度要求 |
//!
//! ## 性能优化
//!
//! ### 模型加载优化
//! - 首次加载模型后保持实例，避免重复加载
//! - 对于长期运行的应用，预加载常用模型
//! - 使用模型缓存减少启动时间
//!
//! ### 音频处理优化
//! - 启用 VAD (语音活动检测) 跳过静音部分
//! - 预转换音频为 Whisper 兼容格式
//! - 批量处理多个文件减少初始化开销
//!
//! ### 系统资源优化
//! - 启用 GPU 加速 (CUDA/Vulkan/Metal)
//! - 调整线程数以优化 CPU 使用率
//! - 监控内存使用，避免大文件处理时的内存溢出
//!
//! ## 错误处理
//!
//! 模块提供了详细的错误类型，帮助快速定位问题：
//!
//! ```rust
//! use rs_voice_toolkit_stt::{SttError, transcribe_file};
//!
//! match transcribe_file("model.bin", "audio.wav").await {
//!     Ok(result) => println!("转录成功: {}", result.text),
//!     Err(SttError::ModelLoadError(e)) => println!("模型加载失败: {}", e),
//!     Err(SttError::AudioProcessingError(e)) => println!("音频处理失败: {}", e),
//!     Err(SttError::WhisperError(e)) => println!("Whisper 处理失败: {}", e),
//!     Err(SttError::IoError(e)) => println!("IO 错误: {}", e),
//!     Err(e) => println!("其他错误: {}", e),
//! }
//! ```
//!
//! ## 系统要求
//!
//! - **内存**:
//!   - tiny 模型: ~200MB
//!   - base 模型: ~400MB
//!   - small 模型: ~800MB
//!   - medium 模型: ~1.5GB
//!   - large 模型: ~3GB
//!
//! - **CPU**: 支持多线程处理，推荐 4 核以上
//! - **GPU**: 可选，支持 CUDA/Vulkan/Metal 加速
//! - **磁盘**: 模型文件存储空间
//!
//! ## 注意事项
//!
//! - 首次使用需要下载 Whisper 模型文件
//! - 建议在使用前验证音频文件格式
//! - 长音频文件建议使用流式处理
//! - 实时应用建议使用 tiny 或 base 模型

// 模块导出集中于下方；避免未使用导入

// 导入错误处理模块
pub mod error;
pub use error::{SttError, SttResult};

// 导入音频处理模块
pub mod audio;
pub use audio::{AudioConfig, AudioData, AudioFormat};

// 导入Whisper转录模块
pub mod whisper;
pub use whisper::{
    TranscriptionResult, TranscriptionSegment, WhisperConfig, WhisperTranscriber, transcribe_file,
    transcribe_file_with_config, transcribe_file_with_language, transcribe_file_with_transcriber,
};

// 导入VAD模块
pub mod vad;
pub use vad::SimpleVad;

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_end_to_end_transcription_on_fixture() {
        // 定位 fixtures 模型与音频
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root_dir = crate_dir; // fixtures 已自包含在 stt crate 内（vendor 时连带改动）
        let model = root_dir.join("fixtures/models/ggml-tiny.bin");
        let audio = root_dir.join("fixtures/audio/jfk.wav");

        if !model.exists() || !audio.exists() {
            eprintln!(
                "跳过: 缺少 fixtures 模型或音频 ({} , {})",
                model.display(),
                audio.display()
            );
            return;
        }

        let result = transcribe_file(&model, &audio)
            .await
            .expect("端到端转录应成功");

        assert!(!result.text.trim().is_empty(), "应产生非空文本");
        assert!(result.audio_duration > 0);
    }

    #[tokio::test]
    async fn test_transcription_bank_audio() {
        // 定位 fixtures 模型与新增的bank_audio.m4a音频文件
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root_dir = crate_dir; // fixtures 已自包含在 stt crate 内（vendor 时连带改动）
        // 测试多个模型
        let models = ["ggml-tiny.bin", "ggml-small.bin", "ggml-medium.bin"];
        let audio = root_dir.join("fixtures/audio/bank_audio.m4a");

        if !audio.exists() {
            eprintln!("跳过: 缺少音频文件: {}", audio.display());
            return;
        }

        for model_name in models {
            let model = root_dir.join("fixtures/models/").join(model_name);

            if !model.exists() {
                println!("跳过: 缺少模型文件: {}", model.display());
                continue;
            }

            println!(
                "\n开始测试bank_audio.m4a文件的转录，使用模型: {}",
                model.display()
            );

            // 方法1: 使用默认配置
            let default_result = transcribe_file(&model, &audio).await;
            println!(
                "默认配置结果: {}",
                default_result
                    .as_ref()
                    .map(|r| &r.text)
                    .unwrap_or(&String::from("失败"))
            );

            // 方法2: 明确指定语言为中文
            let with_lang_result = transcribe_file_with_language(&model, &audio, "zh").await;
            println!(
                "指定中文结果: {}",
                with_lang_result
                    .as_ref()
                    .map(|r| &r.text)
                    .unwrap_or(&String::from("失败"))
            );

            // 方法3: 自定义配置 - 降低置信度要求，适合不太清晰的音频
            let custom_config = WhisperConfig::new(&model)
                .with_language("zh")
                .with_temperature(0.2) // 增加温度可能提高识别率
                .with_vad(false); // 禁用VAD可能有助于捕获所有语音

            let custom_result =
                transcribe_file_with_config(&model, &audio, Some(custom_config)).await;
            println!(
                "自定义配置结果: {}",
                custom_result
                    .as_ref()
                    .map(|r| &r.text)
                    .unwrap_or(&String::from("失败"))
            );
        }
    }

    #[tokio::test]
    async fn test_different_models_on_bank_audio() {
        // 测试不同模型在同一音频上的表现
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root_dir = crate_dir; // fixtures 已自包含在 stt crate 内（vendor 时连带改动）
        let audio = root_dir.join("fixtures/audio/bank_audio.m4a");

        if !audio.exists() {
            eprintln!("跳过: 缺少音频文件: {}", audio.display());
            return;
        }

        // 尝试的模型列表
        let models_to_test = ["ggml-tiny.bin", "ggml-small.bin", "ggml-medium.bin"];

        for model_name in models_to_test {
            let model = root_dir.join("fixtures/models/").join(model_name);

            if !model.exists() {
                println!("跳过: 缺少模型文件: {}", model.display());
                continue;
            }

            println!("\n测试模型: {}", model_name);
            match transcribe_file(&model, &audio).await {
                Ok(result) => {
                    println!("  转录结果: {}", result.text);
                    println!("  音频时长: {}毫秒", result.audio_duration);
                    println!("  处理时长: {}毫秒", result.processing_time);
                    println!("  实时因子: {:.2}x", result.real_time_factor());
                    println!("  检测到的语言: {:?}", result.language);
                    println!("  分段数量: {}", result.segments.len());
                }
                Err(err) => {
                    println!("  转录失败: {}", err);
                }
            }
        }
    }
}

// 导入流式转录模块
#[cfg(feature = "streaming")]
pub mod streaming;
#[cfg(feature = "streaming")]
pub use streaming::{
    StreamingConfig, StreamingEvent, StreamingTranscriber, create_custom_streaming_transcriber,
    create_streaming_transcriber,
};
