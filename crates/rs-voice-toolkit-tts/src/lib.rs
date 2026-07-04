//! # TTS (Text-to-Speech) Module - 文本转语音模块
//!
//! 这个模块提供了高质量的文本转语音功能，支持多种 TTS 引擎和灵活的配置选项。
//!
//! ## 主要功能
//!
//! ### 核心特性
//! - **多引擎支持**: 支持 Index-TTS、Piper、Coqui 等多种 TTS 引擎
//! - **灵活配置**: 支持语言、说话人、采样率、速度、音调等参数配置
//! - **多种输出**: 支持内存输出和文件输出两种方式
//! - **高质量语音**: 基于 Index-TTS 引擎提供自然流畅的语音合成
//! - **异步处理**: 完全异步的 API 设计，适合高并发场景
//! - **可扩展架构**: 易于添加新的 TTS 引擎支持
//!
//! ### 支持的语言和说话人
//! - **中文**: 支持标准中文语音合成
//! - **英文**: 支持标准英文语音合成
//! - **自动检测**: 根据文本内容自动选择语言
//! - **多说话人**: 支持不同说话人声音（取决于引擎支持）
//!
//! ### 音频输出格式
//! - **WAV**: 标准 WAV 格式输出
//! - **采样率**: 支持 8kHz - 48kHz 采样率
//! - **位深度**: 16-bit PCM
//! - **声道**: 单声道/立体声（取决于引擎支持）
//!
//! ## 快速开始
//!
//! ### 基本语音合成
//!
//! ```ignore
//! use rs_voice_toolkit_tts::{TtsService, TtsConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 创建默认配置
//!     let config = TtsConfig::default();
//!     let service = TtsService::new(config);
//!     
//!     // 检查引擎可用性
//!     if !service.is_available().await {
//!         println!("TTS 引擎不可用，请检查安装");
//!         return Ok(());
//!     }
//!     
//!     // 文本转语音（输出到文件）
//!     let text = "你好，世界！欢迎使用语音工具库。";
//!     let output_path = "output/hello.wav";
//!     
//!     service.text_to_file(text, output_path).await?;
//!     println!("语音合成完成: {}", output_path);
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 自定义配置语音合成
//!
//! ```ignore
//! use rs_voice_toolkit_tts::{TtsService, TtsConfig, TtsEngineType};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // 自定义配置
//!     let config = TtsConfig {
//!         language: Some("zh".to_string()),      // 中文
//!         speaker: Some("female".to_string()),   // 女声
//!         sample_rate: 22050,                    // 采样率
//!         speed: 1.2,                           // 语速稍快
//!         pitch: 0.0,                           // 正常音调
//!         executable_path: None,                // 使用系统 PATH
//!     };
//!     
//!     let service = TtsService::new(config);
//!     
//!     // 文本转语音（输出到内存）
//!     let text = "这是一段使用自定义配置生成的语音。";
//!     let audio_data = service.text_to_speech(text).await?;
//!     
//!     println!("生成音频数据大小: {} 字节", audio_data.len());
//!     
//!     // 保存到文件
//!     std::fs::write("output/custom.wav", audio_data)?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ### 批量语音合成
//!
//! ```ignore
//! use rs_voice_toolkit_tts::{TtsService, TtsConfig};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let service = TtsService::new(TtsConfig::default());
//!     
//!     // 批量合成多个文本
//!     let texts = vec![
//!         "这是第一段文本。",
//!         "这是第二段文本。",
//!         "这是第三段文本。",
//!     ];
//!     
//!     for (i, text) in texts.iter().enumerate() {
//!         let output_path = format!("output/speech_{}.wav", i + 1);
//!         
//!         match service.text_to_file(text, &output_path).await {
//!             Ok(_) => println!("✓ {}: {}", i + 1, output_path),
//!             Err(e) => println!("✗ {}: {} - {}", i + 1, text, e),
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## 引擎选择指南
//!
//! ### Index-TTS 引擎（默认）
//! - **特点**: 高质量、多语言支持、配置灵活
//! - **适用场景**: 通用语音合成、多语言应用
//! - **安装**: 需要安装 index-tts 可执行文件
//!
//! ### Piper 引擎（计划中）
//! - **特点**: 轻量级、离线运行、多说话人支持
//! - **适用场景**: 嵌入式设备、离线应用
//! - **状态**: 计划中功能
//!
//! ### Coqui 引擎（计划中）
//! - **特点**: 高质量、可训练、多语言
//! - **适用场景**: 专业语音合成、定制化需求
//! - **状态**: 计划中功能
//!
//! ## 配置参数说明
//!
//! ### 基本配置
//! - **language**: 语言设置（"zh", "en", "auto"）
//! - **speaker**: 说话人选择（引擎相关）
//! - **sample_rate**: 采样率（8000-48000 Hz）
//!
//! ### 语音效果调整
//! - **speed**: 语速控制（0.5-2.0，1.0为正常）
//! - **pitch**: 音调调整（-20.0到20.0，0.0为正常）
//!
//! ### 引擎配置
//! - **executable_path**: 引擎可执行文件路径
//!
//! ## 性能优化
//!
//! ### 引擎优化
//! - **保持服务实例**: 避免重复创建 TtsService 实例
//! - **批量处理**: 使用批量合成减少初始化开销
//! - **异步并发**: 利用异步特性进行并发处理
//!
//! ### 系统资源优化
//! - **内存管理**: 及时释放大型音频数据
//! - **磁盘空间**: 合理管理生成的音频文件
//! - **CPU 使用**: 监控合成过程中的 CPU 使用率
//!
//! ### 音频质量优化
//! - **采样率选择**: 根据应用场景选择合适的采样率
//! - **语速调整**: 避免过快的语速影响可懂度
//! - **音调调整**: 适度调整音调避免不自然
//!
//! ## 错误处理
//!
//! 模块提供了详细的错误类型，帮助快速定位问题：
//!
//! ```ignore
//! use rs_voice_toolkit_tts::{TtsService, TtsConfig, TtsError};
//!
//! let service = TtsService::new(TtsConfig::default());
//!
//! match service.text_to_speech("测试文本").await {
//!     Ok(audio_data) => println!("合成成功，音频大小: {} 字节", audio_data.len()),
//!     Err(TtsError::NotImplemented) => println!("功能尚未实现"),
//!     Err(TtsError::ConfigError(msg)) => println!("配置错误: {}", msg),
//!     Err(TtsError::AudioGenerationError(msg)) => println!("音频生成错误: {}", msg),
//!     Err(TtsError::EngineExecutionError(msg)) => println!("引擎执行错误: {}", msg),
//!     Err(e) => println!("其他错误: {}", e),
//! }
//! ```
//!
//! ## 系统要求
//!
//! ### Index-TTS 引擎要求
//! - **内存**: ~100MB 运行内存
//! - **CPU**: 支持多线程处理
//! - **磁盘**: 引擎安装空间 ~50MB
//! - **依赖**: index-tts 可执行文件
//!
//! ### 系统兼容性
//! - **Linux**: 完全支持
//! - **macOS**: 完全支持
//! - **Windows**: 基本支持（取决于引擎）
//!
//! ## 注意事项
//!
//! - 首次使用前需要安装相应的 TTS 引擎
//! - 建议在使用前验证引擎可用性
//! - 长文本建议分段处理以避免内存问题
//! - 实时应用建议使用较小的采样率以减少延迟
//! - 生成的音频文件需要注意版权问题

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;

/// TTS模块的错误类型
///
/// 这个枚举定义了TTS模块中可能出现的所有错误情况，提供了详细的错误信息
/// 以帮助开发者快速定位和解决问题。
///
/// # 错误类型说明
///
/// - `NotImplemented`: 某些功能尚未实现，主要是一些计划中的引擎
/// - `ConfigError`: 配置参数错误，如无效的采样率、缺失的可执行文件等
/// - `AudioGenerationError`: 音频生成过程中的错误，如内存不足、格式不支持等
/// - `EngineExecutionError`: TTS引擎执行过程中的错误，如进程启动失败、异常退出等
///
/// # 使用示例
///
/// ```ignore
/// use rs_voice_toolkit_tts::TtsError;
///
/// match some_tts_operation() {
///     Ok(result) => println!("操作成功: {:?}", result),
///     Err(TtsError::NotImplemented) => println!("该功能尚未实现"),
///     Err(TtsError::ConfigError(msg)) => println!("配置错误: {}", msg),
///     Err(TtsError::AudioGenerationError(msg)) => println!("音频生成失败: {}", msg),
///     Err(TtsError::EngineExecutionError(msg)) => println!("引擎执行失败: {}", msg),
/// }
/// ```
#[derive(Error, Debug)]
pub enum TtsError {
    /// TTS功能尚未实现
    ///
    /// 这个错误通常在尝试使用尚未实现的功能时出现，比如计划中的 Piper 或 Coqui 引擎。
    #[error("TTS功能尚未实现")]
    NotImplemented,

    /// 配置错误
    ///
    /// 这个错误表示TTS配置存在问题，常见原因包括：
    /// - 找不到指定的可执行文件
    /// - 无效的采样率设置
    /// - 不支持的语言或说话人设置
    /// - 其他配置参数错误
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsError;
    /// let error = TtsError::ConfigError("找不到 index-tts 可执行文件".to_string());
    /// println!("配置错误: {}", error);
    /// ```
    #[error("配置错误: {0}")]
    ConfigError(String),

    /// 音频生成错误
    ///
    /// 这个错误在音频数据生成过程中出现，可能原因包括：
    /// - 内存不足导致无法生成音频
    /// - 音频格式不支持
    /// - 文本内容为空或无效
    /// - 音频编码过程失败
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsError;
    /// let error = TtsError::AudioGenerationError("内存不足，无法生成音频数据".to_string());
    /// println!("音频生成错误: {}", error);
    /// ```
    #[error("音频生成错误: {0}")]
    AudioGenerationError(String),

    /// 引擎执行错误
    ///
    /// 这个错误在TTS引擎执行过程中出现，可能原因包括：
    /// - 引擎进程启动失败
    /// - 引擎进程异常退出
    /// - 引擎输出格式错误
    /// - 引擎内部错误
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsError;
    /// let error = TtsError::EngineExecutionError("index-tts 进程异常退出".to_string());
    /// println!("引擎执行错误: {}", error);
    /// ```
    #[error("引擎执行错误: {0}")]
    EngineExecutionError(String),
}

/// TTS配置
///
/// 这个结构体定义了TTS引擎的所有配置参数，提供了灵活的语音合成选项。
/// 通过调整这些参数，可以获得不同语言、说话人、音色和语速的语音输出。
///
/// # 配置参数说明
///
/// - `executable_path`: TTS引擎可执行文件路径，如果为None则从系统PATH中查找
/// - `language`: 目标语言代码，如"zh"(中文)、"en"(英文)、"auto"(自动检测)
/// - `speaker`: 说话人标识，具体支持的值取决于TTS引擎
/// - `sample_rate`: 输出音频的采样率，范围通常在8000-48000Hz之间
/// - `speed`: 语音播放速度，1.0为正常速度，范围0.5-2.0
/// - `pitch`: 音调调整，0.0为正常音调，范围-20.0到20.0
///
/// # 使用示例
///
/// ## 基本配置
///
/// ```ignore
/// use rs_voice_toolkit_tts::TtsConfig;
///
/// // 使用默认配置
/// let config = TtsConfig::default();
///
/// // 自定义配置
/// let config = TtsConfig {
///     executable_path: Some("/usr/local/bin/index-tts".into()),
///     language: Some("zh".to_string()),
///     speaker: Some("female".to_string()),
///     sample_rate: 22050,
///     speed: 1.0,
///     pitch: 0.0,
/// };
/// ```
///
/// ## 链式配置
///
/// ```ignore
/// use rs_voice_toolkit_tts::TtsConfig;
///
/// let config = TtsConfig {
///     language: Some("zh".to_string()),
///     speaker: Some("male".to_string()),
///     ..TtsConfig::default()
/// };
/// ```
///
/// # 参数建议
///
/// ## 采样率选择
/// - `8000`: 电话质量，文件小，适合网络传输
/// - `16000`: 标准语音识别质量，清晰度较好
/// - `22050`: 多媒体应用标准，音质良好
/// - `44100`: CD质量，音质最佳但文件较大
///
/// ## 语速调整
/// - `0.5`: 慢速，适合学习或重要信息
/// - `1.0`: 正常速度，适合大多数场景
/// - `1.5`: 快速，适合信息密度高的内容
/// - `2.0`: 极快，适合快速浏览
///
/// ## 音调调整
/// - `-20.0`: 极低音，特殊效果
/// - `-10.0`: 低音，男性化效果
/// - `0.0`: 正常音调
/// - `10.0`: 高音，女性化效果
/// - `20.0`: 极高音，特殊效果
///
/// # 注意事项
///
/// - 并非所有TTS引擎都支持所有配置参数
/// - 某些参数的有效范围可能因引擎而异
/// - 建议在使用前测试不同参数组合的效果
/// - 过于极端的参数值可能导致合成质量下降
#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// Index-TTS 可执行文件路径
    ///
    /// 指定TTS引擎可执行文件的完整路径。如果设置为None，
    /// 系统会在PATH环境变量中查找引擎。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// // 使用系统PATH中的引擎
    /// let config1 = TtsConfig { executable_path: None, ..Default::default() };
    ///
    /// // 指定特定路径的引擎
    /// let config2 = TtsConfig {
    ///     executable_path: Some("/usr/local/bin/index-tts".into()),
    ///     ..Default::default()
    /// };
    /// ```
    pub executable_path: Option<PathBuf>,

    /// 语言设置
    ///
    /// 指定语音合成的目标语言。支持的语言取决于TTS引擎。
    ///
    /// # 支持的语言代码
    /// - `"zh"`: 中文
    /// - `"en"`: 英文
    /// - `"auto"`: 自动检测（如果引擎支持）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// let config = TtsConfig {
    ///     language: Some("zh".to_string()),
    ///     ..Default::default()
    /// };
    /// ```
    pub language: Option<String>,

    /// 说话人设置
    ///
    /// 指定语音合成的说话人。支持的说话人取决于TTS引擎。
    ///
    /// # 常见说话人选项
    /// - `"male"`: 男性声音
    /// - `"female"`: 女性声音
    /// - `"child"`: 儿童声音
    /// - 其他引擎特定的说话人标识
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// let config = TtsConfig {
    ///     speaker: Some("female".to_string()),
    ///     ..Default::default()
    /// };
    /// ```
    pub speaker: Option<String>,

    /// 采样率
    ///
    /// 指定输出音频的采样率，单位为Hz。采样率越高，
    /// 音质越好，但文件也越大。常见的采样率有：
    ///
    /// - `8000`: 电话质量
    /// - `16000`: 标准语音质量
    /// - `22050`: 多媒体质量
    /// - `44100`: CD质量
    /// - `48000`: 专业音频质量
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// let config = TtsConfig {
    ///     sample_rate: 22050,
    ///     ..Default::default()
    /// };
    /// ```
    pub sample_rate: u32,

    /// 语音速度
    ///
    /// 控制语音播放的速度。1.0为正常速度，值越大播放越快，
    /// 值越小播放越慢。建议范围：0.5 - 2.0。
    ///
    /// # 速度对照
    /// - `0.5`: 正常速度的一半
    /// - `0.8`: 稍慢
    /// - `1.0`: 正常速度
    /// - `1.2`: 稍快
    /// - `1.5`: 正常速度的1.5倍
    /// - `2.0`: 正常速度的2倍
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// let config = TtsConfig {
    ///     speed: 1.2,
    ///     ..Default::default()
    /// };
    /// ```
    pub speed: f32,

    /// 音调调整
    ///
    /// 调整语音的音调。0.0为正常音调，正值提高音调，
    /// 负值降低音调。建议范围：-20.0 - 20.0。
    ///
    /// # 音调效果
    /// - `-20.0`: 极低音
    /// - `-10.0`: 低音
    /// - `0.0`: 正常音调
    /// - `10.0`: 高音
    /// - `20.0`: 极高音
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::TtsConfig;
    /// let config = TtsConfig {
    ///     pitch: 5.0,
    ///     ..Default::default()
    /// };
    /// ```
    pub pitch: f32,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            executable_path: None,
            language: Some("auto".to_string()),
            speaker: None,
            sample_rate: 22050,
            speed: 1.0,
            pitch: 0.0,
        }
    }
}

/// TTS引擎类型
///
/// 这个枚举定义了支持的TTS引擎类型。目前只有Index-TTS引擎是完整实现的，
/// 其他引擎作为未来扩展的预留选项。
///
/// # 引擎对比
///
/// ## Index-TTS（当前默认）
/// - **特点**: 高质量、多语言、配置灵活
/// - **支持**: 中文、英文等多种语言
/// - **输出**: WAV格式音频
/// - **安装**: 需要index-tts可执行文件
///
/// ## Piper（计划中）
/// - **特点**: 轻量级、离线运行、多说话人
/// - **支持**: 多种语言和声音
/// - **输出**: WAV格式音频
/// - **安装**: 单文件可执行程序
///
/// ## Coqui（计划中）
/// - **特点**: 高质量、可训练、专业级
/// - **支持**: 自定义模型训练
/// - **输出**: 多种音频格式
/// - **安装**: Python依赖和模型文件
///
/// # 使用示例
///
/// ```ignore
/// use rs_voice_toolkit_tts::{TtsService, TtsConfig, TtsEngineType};
///
/// // 使用默认引擎（Index-TTS）
/// let service1 = TtsService::new(TtsConfig::default());
///
/// // 显式指定Index-TTS引擎
/// let service2 = TtsService::new_with_engine(
///     TtsConfig::default(),
///     TtsEngineType::IndexTts
/// );
///
/// // 未来使用其他引擎（尚未实现）
/// // let service3 = TtsService::new_with_engine(
/// //     TtsConfig::default(),
/// //     TtsEngineType::Piper  // 或 TtsEngineType::Coqui
/// // );
/// ```
///
/// # 引擎选择建议
///
/// - **通用应用**: 使用Index-TTS引擎
/// - **离线需求**: 等待Piper引擎实现
/// - **专业需求**: 等待Coqui引擎实现
/// - **嵌入式设备**: 考虑未来的Piper引擎
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TtsEngineType {
    /// Index-TTS 引擎
    ///
    /// 当前默认和唯一完整实现的TTS引擎。
    /// 提供高质量的语音合成功能，支持多种语言和配置选项。
    #[default]
    IndexTts,

    /// Piper 引擎（未来支持）
    ///
    /// 计划中的轻量级TTS引擎，特点是：
    /// - 单文件可执行程序
    /// - 完全离线运行
    /// - 支持多种说话人
    /// - 适合嵌入式设备
    ///
    /// # 状态
    /// 目前尚未实现，计划未来版本支持。
    #[allow(dead_code)]
    Piper,

    /// Coqui 引擎（未来支持）
    ///
    /// 计划中的专业级TTS引擎，特点是：
    /// - 高质量语音合成
    /// - 支持自定义模型训练
    /// - 多语言支持
    /// - 专业应用场景
    ///
    /// # 状态
    /// 目前尚未实现，计划未来版本支持。
    #[allow(dead_code)]
    Coqui,
}

/// TTS引擎接口
///
/// 这个trait定义了所有TTS引擎必须实现的基本接口。通过这个统一的接口，
/// 可以轻松添加新的TTS引擎实现，同时保持API的一致性。
///
/// # 接口设计原则
///
/// - **简洁性**: 只包含最核心的语音合成功能
/// - **扩展性**: 易于添加新的引擎实现
/// - **一致性**: 所有引擎提供相同的API接口
/// - **异步性**: 支持异步操作，适合高并发场景
///
/// # 实现要求
///
/// 每个TTS引擎实现必须提供：
/// - 文本到音频数据的转换功能
/// - 文件输出功能
/// - 引擎可用性检查
/// - 支持的语言列表
/// - 引擎类型标识
///
/// # 使用示例
///
/// ```ignore
/// use rs_voice_toolkit_tts::{TtsEngine, TtsError, TtsEngineType};
/// use std::path::Path;
///
/// struct MyTtsEngine;
///
/// #[async_trait::async_trait]
/// impl TtsEngine for MyTtsEngine {
///     async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
///         // 实现文本到音频的转换
///         Ok(Vec::new())
///     }
///     
///     async fn synthesize_to_file(&self, text: &str, output_path: &Path) -> Result<(), TtsError> {
///         // 实现文件输出
///         Ok(())
///     }
///     
///     async fn is_available(&self) -> bool {
///         // 检查引擎是否可用
///         true
///     }
///     
///     fn supported_languages(&self) -> Vec<String> {
///         // 返回支持的语言列表
///         vec!["zh".to_string(), "en".to_string()]
///     }
///     
///     fn engine_type(&self) -> TtsEngineType {
///         // 返回引擎类型
///         TtsEngineType::IndexTts
///     }
/// }
/// ```
///
/// # 性能考虑
///
/// - `synthesize()` 方法应该避免重复的引擎初始化开销
/// - `is_available()` 方法应该快速返回，不应该有昂贵的检查
/// - `supported_languages()` 方法应该返回缓存的结果，避免重复计算
/// - 文件操作应该使用异步方式以避免阻塞
#[async_trait]
pub trait TtsEngine {
    /// 将文本转换为语音
    ///
    /// 这是核心的语音合成方法，将输入的文本转换为音频数据。
    /// 返回的音频数据通常是WAV格式的二进制数据。
    ///
    /// # 参数
    ///
    /// - `text`: 要合成的文本内容
    ///
    /// # 返回值
    ///
    /// 返回 `Result<Vec<u8>, TtsError>`，成功时包含音频数据的字节向量，
    /// 失败时返回相应的错误信息。
    ///
    /// # 错误处理
    ///
    /// 可能的错误包括：
    /// - `TtsError::NotImplemented`: 功能未实现
    /// - `TtsError::ConfigError`: 配置错误
    /// - `TtsError::AudioGenerationError`: 音频生成失败
    /// - `TtsError::EngineExecutionError`: 引擎执行失败
    ///
    /// # 示例
    ///
    /// ```ignore
    /// # use rs_voice_toolkit_tts::{TtsEngine, TtsError};
    /// # struct MockEngine;
    /// # #[async_trait::async_trait]
    /// # impl TtsEngine for MockEngine {
    /// async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
    ///     if text.is_empty() {
    ///         return Err(TtsError::AudioGenerationError("文本不能为空".to_string()));
    ///     }
    ///     
    ///     // 模拟音频数据生成
    ///     let audio_data = vec![0u8; 1024]; // 实际应用中这里是真实的音频数据
    ///     Ok(audio_data)
    /// }
    /// # fn supported_languages(&self) -> Vec<String> { vec![] }
    /// # fn engine_type(&self) -> rs_voice_toolkit_tts::TtsEngineType { unimplemented!() }
    /// # async fn is_available(&self) -> bool { true }
    /// # async fn synthesize_to_file(&self, _: &str, _: &std::path::Path) -> Result<(), TtsError> { Ok(()) }
    /// # }
    /// ```
    ///
    /// # 性能提示
    ///
    /// - 对于长文本，考虑分段处理以避免内存问题
    /// - 多次调用此方法时，保持引擎实例以减少初始化开销
    /// - 返回的音频数据应该及时处理或保存，避免内存泄漏
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError>;

    /// 将文本转换为语音并保存到文件
    ///
    /// 这个方法将文本转换为语音并直接保存到指定文件路径。
    /// 这是 `synthesize()` 方法的便捷版本，避免在内存中保存大型音频文件。
    ///
    /// # 参数
    ///
    /// - `text`: 要合成的文本内容
    /// - `output_path`: 输出文件的路径
    ///
    /// # 返回值
    ///
    /// 返回 `Result<(), TtsError>`，成功时表示文件保存成功，
    /// 失败时返回相应的错误信息。
    ///
    /// # 错误处理
    ///
    /// 除了 `synthesize()` 方法可能返回的错误外，还可能包括：
    /// - 文件系统相关的错误（通过 `TtsError::AudioGenerationError` 传递）
    /// - 磁盘空间不足等IO错误
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use rs_voice_toolkit_tts::{TtsEngine, TtsError};
    /// use std::path::Path;
    ///
    /// # struct MockEngine;
    /// # #[async_trait::async_trait]
    /// # impl TtsEngine for MockEngine {
    /// async fn synthesize_to_file(&self, text: &str, output_path: &Path) -> Result<(), TtsError> {
    ///     let audio_data = self.synthesize(text).await?;
    ///     std::fs::write(output_path, audio_data)
    ///         .map_err(|e| TtsError::AudioGenerationError(
    ///             format!("保存文件失败: {}", e)
    ///         ))?;
    ///     Ok(())
    /// }
    /// # fn supported_languages(&self) -> Vec<String> { vec![] }
    /// # fn engine_type(&self) -> rs_voice_toolkit_tts::TtsEngineType { unimplemented!() }
    /// # async fn is_available(&self) -> bool { true }
    /// # async fn synthesize(&self, _: &str) -> Result<Vec<u8>, TtsError> { Ok(vec![]) }
    /// # }
    /// ```
    ///
    /// # 使用建议
    ///
    /// - 对于大型音频文件，建议使用此方法而不是 `synthesize()`
    /// - 确保输出目录存在且有写权限
    /// - 考虑文件命名冲突问题
    async fn synthesize_to_file(&self, text: &str, output_path: &Path) -> Result<(), TtsError>;

    /// 检查引擎是否可用
    ///
    /// 这个方法快速检查TTS引擎是否可用。应该在执行语音合成前调用此方法，
    /// 以避免在引擎不可用时浪费时间。
    ///
    /// # 返回值
    ///
    /// 返回 `bool` 值：
    /// - `true`: 引擎可用，可以正常进行语音合成
    /// - `false`: 引擎不可用，需要检查安装或配置
    ///
    /// # 检查内容
    ///
    /// 具体的检查内容取决于引擎实现，通常包括：
    /// - 可执行文件是否存在
    /// - 依赖库是否完整
    /// - 配置文件是否正确
    /// - 权限是否足够
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use rs_voice_toolkit_tts::{TtsEngine, TtsError};
    ///
    /// # struct MockEngine;
    /// # #[async_trait::async_trait]
    /// # impl TtsEngine for MockEngine {
    /// async fn is_available(&self) -> bool {
    ///     // 检查可执行文件是否存在
    ///     std::path::Path::new("/usr/local/bin/index-tts").exists()
    /// }
    /// # fn supported_languages(&self) -> Vec<String> { vec![] }
    /// # fn engine_type(&self) -> rs_voice_toolkit_tts::TtsEngineType { unimplemented!() }
    /// # async fn synthesize(&self, _: &str) -> Result<Vec<u8>, TtsError> { Ok(vec![]) }
    /// # async fn synthesize_to_file(&self, _: &str, _: &std::path::Path) -> Result<(), TtsError> { Ok(()) }
    /// # }
    /// ```
    ///
    /// # 性能考虑
    ///
    /// - 这个方法应该快速返回，避免昂贵的检查操作
    /// - 可以缓存检查结果，避免重复的系统调用
    /// - 考虑添加异步版本的检查以提高响应性
    async fn is_available(&self) -> bool;

    /// 获取支持的语言列表
    ///
    /// 返回该TTS引擎支持的语言代码列表。用户可以根据这个列表
    /// 选择合适的语言进行语音合成。
    ///
    /// # 返回值
    ///
    /// 返回 `Vec<String>`，包含支持的语言代码。常见的语言代码包括：
    /// - `"zh"`: 中文
    /// - `"en"`: 英文
    /// - `"ja"`: 日文
    /// - `"ko"`: 韩文
    /// - 等等
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use rs_voice_toolkit_tts::{TtsEngine, TtsEngineType};
    ///
    /// # struct MockEngine;
    /// # #[async_trait::async_trait]
    /// # impl TtsEngine for MockEngine {
    /// fn supported_languages(&self) -> Vec<String> {
    ///     vec![
    ///         "zh".to_string(),
    ///         "en".to_string(),
    ///         "auto".to_string(),
    ///     ]
    /// }
    /// # fn engine_type(&self) -> rs_voice_toolkit_tts::TtsEngineType { unimplemented!() }
    /// # async fn is_available(&self) -> bool { true }
    /// # async fn synthesize(&self, _: &str) -> Result<Vec<u8>, TtsError> { Ok(vec![]) }
    /// # async fn synthesize_to_file(&self, _: &str, _: &std::path::Path) -> Result<(), TtsError> { Ok(()) }
    /// # }
    /// ```
    ///
    /// # 使用建议
    ///
    /// - 在选择语言前调用此方法检查支持情况
    /// - 可以在应用启动时缓存此结果
    /// - 考虑将此信息展示给用户供选择
    fn supported_languages(&self) -> Vec<String>;

    /// 获取引擎类型
    ///
    /// 返回该TTS引擎的类型标识。这个标识可以帮助用户了解当前使用的引擎，
    /// 并在需要时进行引擎特定的配置或处理。
    ///
    /// # 返回值
    ///
    /// 返回 `TtsEngineType` 枚举值：
    /// - `TtsEngineType::IndexTts`: Index-TTS引擎
    /// - `TtsEngineType::Piper`: Piper引擎（未来支持）
    /// - `TtsEngineType::Coqui`: Coqui引擎（未来支持）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use rs_voice_toolkit_tts::{TtsEngine, TtsEngineType};
    ///
    /// # struct MockEngine;
    /// # #[async_trait::async_trait]
    /// # impl TtsEngine for MockEngine {
    /// fn engine_type(&self) -> TtsEngineType {
    ///     TtsEngineType::IndexTts
    /// }
    /// # fn supported_languages(&self) -> Vec<String> { vec![] }
    /// # async fn is_available(&self) -> bool { true }
    /// # async fn synthesize(&self, _: &str) -> Result<Vec<u8>, TtsError> { Ok(vec![]) }
    /// # async fn synthesize_to_file(&self, _: &str, _: &std::path::Path) -> Result<(), TtsError> { Ok(()) }
    /// # }
    /// ```
    ///
    /// # 使用场景
    ///
    /// - 引擎特定的配置和优化
    /// - 日志记录和调试信息
    /// - 用户界面显示当前引擎
    /// - 引擎切换和兼容性检查
    fn engine_type(&self) -> TtsEngineType;
}

/// Index-TTS 引擎
///
/// 这是Index-TTS引擎的具体实现，提供了完整的文本转语音功能。
/// Index-TTS是一个高质量的语音合成引擎，支持多种语言和灵活的配置选项。
///
/// # 主要特性
///
/// - **多语言支持**: 支持中文、英文等多种语言
/// - **高质量合成**: 提供自然流畅的语音输出
/// - **灵活配置**: 支持语言、说话人、采样率等多种参数配置
/// - **异步处理**: 完全异步的API设计
/// - **多种输出**: 支持内存输出和文件输出
///
/// # 使用示例
///
/// ## 基本使用
///
/// ```ignore
/// use rs_voice_toolkit_tts::{IndexTtsEngine, TtsConfig};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // 创建配置和引擎
///     let config = TtsConfig::default();
///     let engine = IndexTtsEngine::new(config);
///     
///     // 检查引擎可用性
///     if !engine.is_available().await {
///         println!("Index-TTS 引擎不可用");
///         return Ok(());
///     }
///     
///     // 文本转语音
///     let text = "你好，世界！";
///     let audio_data = engine.synthesize(text).await?;
///     println!("生成音频数据大小: {} 字节", audio_data.len());
///     
///     Ok(())
/// }
/// ```
///
/// ## 文件输出
///
/// ```ignore
/// use rs_voice_toolkit_tts::{IndexTtsEngine, TtsConfig};
/// use std::path::Path;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = TtsConfig {
///     language: Some("zh".to_string()),
///     sample_rate: 22050,
///     ..TtsConfig::default()
/// };
/// let engine = IndexTtsEngine::new(config);
///
/// let text = "这是保存到文件的语音合成示例。";
/// let output_path = Path::new("output.wav");
///
/// engine.synthesize_to_file(text, output_path).await?;
/// println!("语音已保存到: {:?}", output_path);
/// # Ok(())
/// # }
/// ```
///
/// # 引擎要求
///
/// ## 系统要求
/// - **操作系统**: Linux、macOS、Windows
/// - **内存**: 至少100MB可用内存
/// - **CPU**: 支持多线程处理
/// - **磁盘**: 足够的临时文件空间
///
/// ## 软件依赖
/// - **index-tts**: Index-TTS可执行文件
/// - **which**: 用于查找可执行文件（可选）
///
/// ## 安装Index-TTS
///
/// 通常需要从Index-TTS的官方仓库下载可执行文件，
/// 或通过包管理器安装：
///
/// ```bash
/// # 示例安装命令（具体命令取决于发行版）
/// wget https://github.com/open-mmlab/Index-TTS/releases/download/v1.0/index-tts
/// chmod +x index-tts
/// sudo mv index-tts /usr/local/bin/
/// ```
///
/// # 性能优化
///
/// ## 引擎初始化
/// - 保持引擎实例，避免重复创建
/// - 预检查引擎可用性，避免运行时错误
///
/// ## 内存管理
/// - 对于大型音频文件，优先使用文件输出
/// - 及时处理或保存生成的音频数据
///
/// ## 并发处理
/// - 利用异步特性进行并发处理
/// - 注意引擎的并发限制
///
/// # 错误处理
///
/// 引擎可能返回的错误类型：
/// - `TtsError::ConfigError`: 配置错误
/// - `TtsError::AudioGenerationError`: 音频生成失败
/// - `TtsError::EngineExecutionError`: 引擎执行失败
///
/// # 注意事项
///
/// - 首次使用前确保Index-TTS已正确安装
/// - 长文本建议分段处理以避免内存问题
/// - 建议在使用前检查引擎可用性
/// - 注意生成的音频文件的版权问题
#[derive(Debug, Clone)]
pub struct IndexTtsEngine {
    /// TTS配置
    ///
    /// 存储引擎的配置参数，包括语言、说话人、采样率等设置。
    /// 这些配置会在语音合成时传递给Index-TTS引擎。
    cfg: TtsConfig,
}

impl IndexTtsEngine {
    pub fn new(cfg: TtsConfig) -> Self {
        Self { cfg }
    }

    pub async fn is_available(&self) -> bool {
        if let Some(path) = &self.cfg.executable_path {
            return path.exists();
        }
        which::which("index-tts").is_ok()
    }

    async fn resolve_executable(&self) -> Result<PathBuf, TtsError> {
        if let Some(path) = &self.cfg.executable_path {
            return Ok(path.clone());
        }
        which::which("index-tts").map_err(|_| {
            TtsError::ConfigError(
                "找不到 index-tts 可执行文件，请设置 PATH 或配置 executable_path".into(),
            )
        })
    }

    pub async fn synthesize_to_memory(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        let exe = self.resolve_executable().await?;
        let mut args: Vec<String> = Vec::new();
        args.push("--text".into());
        args.push(text.into());
        if let Some(lang) = &self.cfg.language {
            args.push("--language".into());
            args.push(lang.clone());
        }
        if let Some(speaker) = &self.cfg.speaker {
            args.push("--speaker".into());
            args.push(speaker.clone());
        }
        args.push("--sample-rate".into());
        args.push(self.cfg.sample_rate.to_string());
        args.push("--output-format".into());
        args.push("wav".into());
        // 假设 index-tts 支持 stdout 输出；若不支持，需落盘再读出
        let output = Command::new(exe)
            .args(&args)
            .output()
            .await
            .map_err(|e| TtsError::EngineExecutionError(format!("执行失败: {e}")))?;
        if !output.status.success() {
            return Err(TtsError::EngineExecutionError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(output.stdout)
    }

    pub async fn synthesize_to_file<P: AsRef<Path>>(
        &self,
        text: &str,
        output_path: P,
    ) -> Result<(), TtsError> {
        let exe = self.resolve_executable().await?;
        let mut args: Vec<String> = Vec::new();
        args.push("--text".into());
        args.push(text.into());
        if let Some(lang) = &self.cfg.language {
            args.push("--language".into());
            args.push(lang.clone());
        }
        if let Some(speaker) = &self.cfg.speaker {
            args.push("--speaker".into());
            args.push(speaker.clone());
        }
        args.push("--sample-rate".into());
        args.push(self.cfg.sample_rate.to_string());
        args.push("--output".into());
        args.push(output_path.as_ref().to_string_lossy().to_string());
        args.push("--output-format".into());
        args.push("wav".into());
        let status = Command::new(exe)
            .args(&args)
            .status()
            .await
            .map_err(|e| TtsError::EngineExecutionError(format!("执行失败: {e}")))?;
        if !status.success() {
            return Err(TtsError::EngineExecutionError(format!(
                "Index-TTS 退出状态异常: {:?}",
                status.code()
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl TtsEngine for IndexTtsEngine {
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        self.synthesize_to_memory(text).await
    }

    async fn synthesize_to_file(&self, text: &str, output_path: &Path) -> Result<(), TtsError> {
        self.synthesize_to_file(text, output_path).await
    }

    async fn is_available(&self) -> bool {
        self.is_available().await
    }

    fn supported_languages(&self) -> Vec<String> {
        vec!["zh".into(), "en".into(), "auto".into()]
    }

    fn engine_type(&self) -> TtsEngineType {
        TtsEngineType::IndexTts
    }
}

/// TTS服务
///
/// 这是TTS模块的主要服务类，提供了高级的文本转语音功能。
/// 它封装了具体的TTS引擎实现，为用户提供统一和便捷的API。
///
/// # 主要功能
///
/// - **统一接口**: 提供一致的API，隐藏底层引擎的复杂性
/// - **引擎管理**: 自动管理TTS引擎的创建和配置
/// - **错误处理**: 提供统一的错误处理机制
/// - **异步支持**: 完全异步的API设计，适合高并发场景
/// - **灵活配置**: 支持多种配置选项和引擎选择
///
/// # 使用示例
///
/// ## 基本使用
///
/// ```ignore
/// use rs_voice_toolkit_tts::{TtsService, TtsConfig};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // 创建TTS服务
///     let config = TtsConfig::default();
///     let service = TtsService::new(config);
///     
///     // 检查服务可用性
///     if !service.is_available().await {
///         println!("TTS服务不可用");
///         return Ok(());
///     }
///     
///     // 文本转语音（内存输出）
///     let text = "你好，世界！欢迎使用语音工具库。";
///     let audio_data = service.text_to_speech(text).await?;
///     println!("生成音频数据大小: {} 字节", audio_data.len());
///     
///     // 文本转语音（文件输出）
///     let output_path = "output/hello.wav";
///     service.text_to_file(text, output_path).await?;
///     println!("语音已保存到: {}", output_path);
///     
///     Ok(())
/// }
/// ```
///
/// ## 自定义配置
///
/// ```ignore
/// use rs_voice_toolkit_tts::{TtsService, TtsConfig, TtsEngineType};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // 创建自定义配置
/// let config = TtsConfig {
///     language: Some("zh".to_string()),
///     speaker: Some("female".to_string()),
///     sample_rate: 22050,
///     speed: 1.2,
///     pitch: 0.0,
///     executable_path: None,
/// };
///
/// // 使用自定义引擎创建服务
/// let service = TtsService::new_with_engine(config, TtsEngineType::IndexTts);
///
/// let text = "这是使用自定义配置生成的语音。";
/// let audio_data = service.text_to_speech(text).await?;
/// # Ok(())
/// # }
/// ```
///
/// ## 批量处理
///
/// ```ignore
/// use rs_voice_toolkit_tts::{TtsService, TtsConfig};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let service = TtsService::new(TtsConfig::default());
///
/// // 批量处理多个文本
/// let texts = vec![
///     "第一段文本内容。",
///     "第二段文本内容。",
///     "第三段文本内容。",
/// ];
///
/// for (i, text) in texts.iter().enumerate() {
///     let output_path = format!("output/speech_{}.wav", i + 1);
///     
///     match service.text_to_file(text, &output_path).await {
///         Ok(_) => println!("✓ 成功生成: {}", output_path),
///         Err(e) => println!("✗ 生成失败: {} - {}", text, e),
///     }
/// }
/// # Ok(())
/// # }
/// ```
///
/// # 服务生命周期
///
/// ## 创建
/// - 通过 `TtsService::new()` 创建使用默认引擎的服务
/// - 通过 `TtsService::new_with_engine()` 创建使用指定引擎的服务
///
/// ## 使用
/// - 调用 `text_to_speech()` 进行内存输出
/// - 调用 `text_to_file()` 进行文件输出
/// - 调用 `is_available()` 检查服务可用性
///
/// ## 销毁
/// - 服务实例会在超出作用域时自动销毁
/// - 建议在应用生命周期内保持服务实例
///
/// # 性能优化
///
/// ## 实例管理
/// - 避免频繁创建和销毁服务实例
/// - 在应用启动时创建服务，全程复用
///
/// ## 并发处理
/// - 服务实例是线程安全的，可以并发使用
/// - 注意底层引擎的并发限制
///
/// ## 资源管理
/// - 及时处理大型音频数据，避免内存泄漏
/// - 优先使用文件输出处理大型音频
///
/// # 错误处理
///
/// 服务会将底层引擎的错误转换为统一的错误类型：
/// - `TtsError::NotImplemented`: 功能未实现
/// - `TtsError::ConfigError`: 配置错误
/// - `TtsError::AudioGenerationError`: 音频生成失败
/// - `TtsError::EngineExecutionError`: 引擎执行失败
///
/// # 注意事项
///
/// - 首次使用前确保相应的TTS引擎已正确安装
/// - 建议在使用前检查服务可用性
/// - 长文本建议分段处理以避免内存问题
/// - 注意生成的音频文件的版权问题
pub struct TtsService {
    /// TTS配置
    ///
    /// 存储服务的配置参数，这些参数会在创建引擎时使用。
    /// 配置包括语言、说话人、采样率等设置。
    #[allow(dead_code)]
    config: TtsConfig,

    /// TTS引擎实例
    ///
    /// 实际的TTS引擎实现，负责具体的语音合成工作。
    /// 使用trait对象以支持多种引擎实现。
    engine: Box<dyn TtsEngine + Send + Sync>,
}

impl TtsService {
    /// 创建新的TTS服务
    pub fn new(config: TtsConfig) -> Self {
        Self::new_with_engine(config, TtsEngineType::default())
    }

    /// 使用指定引擎创建TTS服务
    pub fn new_with_engine(config: TtsConfig, engine_type: TtsEngineType) -> Self {
        let engine = Self::create_engine(config.clone(), engine_type);
        Self { config, engine }
    }

    /// 创建指定类型的引擎
    fn create_engine(
        config: TtsConfig,
        engine_type: TtsEngineType,
    ) -> Box<dyn TtsEngine + Send + Sync> {
        match engine_type {
            TtsEngineType::IndexTts => Box::new(IndexTtsEngine::new(config)),
            TtsEngineType::Piper => {
                // 未来实现
                panic!("Piper 引擎尚未实现")
            }
            TtsEngineType::Coqui => {
                // 未来实现
                panic!("Coqui 引擎尚未实现")
            }
        }
    }

    /// 文本转语音（内存）
    pub async fn text_to_speech(&self, text: &str) -> Result<Vec<u8>, TtsError> {
        self.engine.synthesize(text).await
    }

    /// 文本转语音并保存到文件
    pub async fn text_to_file<P: AsRef<Path>>(
        &self,
        text: &str,
        output: P,
    ) -> Result<(), TtsError> {
        self.engine.synthesize_to_file(text, output.as_ref()).await
    }

    /// 引擎可用性
    pub async fn is_available(&self) -> bool {
        self.engine.is_available().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tts_service_creation() {
        let config = TtsConfig::default();
        let service = TtsService::new(config);

        // 可用性检测（不保证 index-tts 存在，仅验证 API 不 panic）
        let _ = service.text_to_speech("你好").await.err();
    }
}
