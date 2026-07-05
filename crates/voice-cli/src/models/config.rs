use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// 服务器配置
    #[serde(default)]
    pub server: ServerConfig,
    /// Whisper 模型配置
    #[serde(default)]
    pub whisper: WhisperConfig,
    /// 日志配置
    pub logging: LoggingConfig,
    /// 守护进程配置
    pub daemon: DaemonConfig,
    /// 任务管理配置
    #[serde(default)]
    pub task_management: TaskManagementConfig,
    /// TTS配置
    #[serde(default)]
    pub tts: TtsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 服务器监听主机
    pub host: String,
    /// 服务器监听端口
    pub port: u16,
    /// 最大文件大小（字节）
    pub max_file_size: usize,
    /// 是否启用 CORS
    pub cors_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    /// 默认 Whisper 模型
    pub default_model: String,
    /// 模型文件存储目录
    pub models_dir: String,
    /// 是否自动下载模型
    pub auto_download: bool,
    /// 支持的模型列表
    pub supported_models: Vec<String>,
    /// 音频处理配置
    pub audio_processing: AudioProcessingConfig,
    /// Worker 配置
    pub workers: WorkersConfig,
    /// STT 引擎配置（P1：transcribe-rs 引擎池 / GPU 加速）
    #[serde(default)]
    pub engine: SttEngineConfig,
    /// 流式配置（P2 LocalAgreement 2 用，P1 仅占位）
    #[serde(default)]
    pub streaming: StreamingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessingConfig {
    /// 支持的音频格式列表
    pub supported_formats: Vec<String>,
    /// 是否自动转换音频格式
    pub auto_convert: bool,
    /// 音频转换超时时间（秒）
    pub conversion_timeout: u32,
    /// 是否清理临时文件
    pub temp_file_cleanup: bool,
    /// 临时文件保留时间（秒）
    pub temp_file_retention: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkersConfig {
    /// 转录工作线程数
    pub transcription_workers: usize,
    /// 通道缓冲区大小
    pub channel_buffer_size: usize,
    /// Worker 超时时间（秒）
    pub worker_timeout: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// 日志级别
    pub level: String,
    /// 日志目录
    pub log_dir: String,
    /// 最大日志文件大小
    pub max_file_size: String,
    /// 最大日志文件数量
    pub max_files: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// PID 文件路径
    pub pid_file: String,
    /// 日志文件路径
    pub log_file: String,
    /// 工作目录
    pub work_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagementConfig {
    /// 最大并发任务数
    pub max_concurrent_tasks: usize,
    /// SQLite 数据库文件路径
    pub sqlite_db_path: String,
    /// 任务重试次数
    pub retry_attempts: usize,
    /// 任务超时时间（秒）
    pub task_timeout_seconds: u64,
    /// 是否捕获 panic
    pub catch_panic: bool,
    /// 任务保留分钟数
    pub task_retention_minutes: u32,
    /// Sled 数据库路径
    pub sled_db_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsConfig {
    /// 是否启用 TTS（默认 false；模型未放置时启用也会在请求时返回明确错误）
    #[serde(default)]
    pub enabled: bool,
    /// 最大文本长度
    #[serde(default = "default_tts_max_text_length")]
    pub max_text_length: usize,
    /// 支持的音频格式（仅文档/校验用；实际由 sherpa-onnx 合成 + audio_encode 编码）
    #[serde(default = "default_tts_supported_formats")]
    pub supported_formats: Vec<String>,
    /// TTS 引擎配置（sherpa-onnx 引擎池）
    #[serde(default)]
    pub engine: TtsEngineConfig,
    /// TTS 流式配置（P5 WS 用；P3/P4 仅占位）
    #[serde(default)]
    pub streaming: TtsStreamingConfig,
    /// TTS 异步任务 SQLite DB 路径（独立于 STT 的 tasks.db，隔离 apalis storage）
    #[serde(default = "default_tts_tasks_db_path")]
    pub tasks_db_path: String,
}

/// TTS 引擎配置（sherpa-onnx Kokoro）。
///
/// v1 走 CPU（`provider=None`，sherpa-onnx 默认预编译库 CPU-only）；
/// v2 升 GPU 需自编 C++ 库 + 设 `provider="coreml"/"cuda"/"vulkan"`（见 plan §GPU 加速策略）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsEngineConfig {
    /// 引擎池大小：1=单实例串行（CPU 通常最优），>1=N 实例并发（N× 内存）
    #[serde(default = "default_tts_pool_size")]
    pub pool_size: usize,
    /// 加速设备：`cpu`(v1 默认) / `coreml` / `cuda` / `vulkan`（v2 自编库后生效）
    #[serde(default = "default_tts_device")]
    pub device: String,
    /// 默认模型 id（对应 `{models_dir}/{model_id}/`）
    #[serde(default = "default_tts_model")]
    pub default_model: String,
    /// 默认音色 id（Kokoro voices.bin 多 speaker 索引）
    #[serde(default)]
    pub default_sid: i32,
    /// 默认语速（1.0 = 原速）
    #[serde(default = "default_tts_speed")]
    pub default_speed: f32,
    /// 默认时长缩放（model-level；1.0 = 原速）
    #[serde(default = "default_tts_length_scale")]
    pub default_length_scale: f32,
    /// 默认语种（多语 Kokoro v1.0 必需，如 `"mixed"`/`"zh"`/`"en"`；None 时若模型要求 lang 会 C 端 std::exit）
    #[serde(default = "default_tts_lang")]
    pub default_language: Option<String>,
    /// ONNX runtime 线程数（0 = sherpa-onnx 默认）
    #[serde(default = "default_tts_num_threads")]
    pub num_threads: i32,
    /// ONNX EP provider（`None`=CPU；v2 GPU 走 `"coreml"`/`"cuda"`/`"vulkan"`）
    #[serde(default)]
    pub provider: Option<String>,
    /// 模型根目录（`{models_dir}/{model_id}/`）
    #[serde(default = "default_tts_models_dir")]
    pub models_dir: String,
    /// sherpa-onnx C 端 verbose 日志
    #[serde(default)]
    pub debug: bool,
}

/// TTS 流式配置（P5 WS 流式合成用；P3/P4 仅占位）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsStreamingConfig {
    /// WS 空闲超时（秒）
    #[serde(default = "default_tts_idle_timeout")]
    pub idle_timeout_sec: u64,
    /// 单次合成超时（秒，兜底防止 C 调用挂死）
    #[serde(default = "default_tts_synth_timeout")]
    pub synth_timeout_sec: u64,
    /// 默认输出格式（`wav` / `pcm_s16le`）
    #[serde(default = "default_tts_format")]
    pub default_format: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            max_file_size: 200 * 1024 * 1024, // 200MB
            cors_enabled: true,
        }
    }
}

impl Default for WhisperConfig {
    fn default() -> Self {
        Self {
            default_model: "base".to_string(),
            models_dir: "./models".to_string(),
            auto_download: true,
            supported_models: vec![
                "tiny".to_string(),
                "tiny.en".to_string(),
                "base".to_string(),
                "base.en".to_string(),
                "small".to_string(),
                "small.en".to_string(),
                "medium".to_string(),
                "medium.en".to_string(),
                "large-v1".to_string(),
                "large-v2".to_string(),
                "large-v3".to_string(),
            ],
            audio_processing: AudioProcessingConfig::default(),
            workers: WorkersConfig::default(),
            engine: SttEngineConfig::default(),
            streaming: StreamingConfig::default(),
        }
    }
}

/// STT 引擎配置（transcribe-rs 引擎池 + GPU 加速）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SttEngineConfig {
    /// 引擎池大小：1=单实例串行（CPU 通常最优，避免线程超订阅），>1=N 实例并发（N× 内存）
    #[serde(default = "default_stt_pool_size")]
    pub pool_size: usize,
    /// 加速设备：`auto`(默认,平台 GPU) / `cpu` / `gpu` / `metal` / `cuda` / `vulkan`
    #[serde(default = "default_stt_device")]
    pub device: String,
    /// flash attention（默认 true，GPU 下加速；CPU 忽略）
    #[serde(default = "default_bool_true")]
    pub flash_attn: bool,
    /// 解码线程数（0 = whisper.cpp 默认 `min(4, num_cores)`）
    #[serde(default)]
    pub n_threads: i32,
    /// 默认目标语种（BCP-47，如 `"en"`/`"zh"`；`None` = 自动检测）
    #[serde(default)]
    pub default_language: Option<String>,
    /// 默认初始提示（领域上下文，提升专有词 / 风格准确率）
    #[serde(default)]
    pub default_initial_prompt: Option<String>,
}

impl Default for SttEngineConfig {
    fn default() -> Self {
        Self {
            pool_size: default_stt_pool_size(),
            device: default_stt_device(),
            flash_attn: default_bool_true(),
            n_threads: 0,
            default_language: None,
            default_initial_prompt: None,
        }
    }
}

/// STT 流式配置（P2 LocalAgreement 2 真流式用；P1 仅占位）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    /// 解码触发间隔（秒）：audio buffer 每新增此时长触发一次 A/B 双解码
    #[serde(default = "default_decode_interval")]
    pub decode_interval_sec: f32,
    /// 尾部裁剪（秒）：B 解码 = 去掉 buffer 尾部此时长，与 A 取最长公共前缀
    #[serde(default = "default_tail_trim")]
    pub tail_trim_sec: f32,
    /// 前缀连续不回退多少次才 commit（标准 LocalAgreement 2 = 2）
    #[serde(default = "default_min_agree")]
    pub min_agree_count: u32,
    /// WS 空闲超时（秒）：无新数据多久后关闭会话
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_sec: u64,
    /// 单次解码超时（秒）：兜底防止同步 C 调用挂死
    #[serde(default = "default_decode_timeout")]
    pub decode_timeout_sec: u64,
    /// 比较粒度：`auto`(按 language 推断：CJK→char，其余→word) / `char` / `word`
    #[serde(default = "default_granularity")]
    pub compare_granularity: String,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            decode_interval_sec: default_decode_interval(),
            tail_trim_sec: default_tail_trim(),
            min_agree_count: default_min_agree(),
            idle_timeout_sec: default_idle_timeout(),
            decode_timeout_sec: default_decode_timeout(),
            compare_granularity: default_granularity(),
        }
    }
}

fn default_stt_pool_size() -> usize {
    1
}
fn default_stt_device() -> String {
    "auto".to_string()
}
fn default_bool_true() -> bool {
    true
}
fn default_decode_interval() -> f32 {
    0.5
}
fn default_tail_trim() -> f32 {
    0.3
}
fn default_min_agree() -> u32 {
    2
}
fn default_idle_timeout() -> u64 {
    30
}
fn default_decode_timeout() -> u64 {
    30
}
fn default_granularity() -> String {
    "auto".to_string()
}

impl Default for AudioProcessingConfig {
    fn default() -> Self {
        Self {
            supported_formats: vec![
                "mp3".to_string(),
                "wav".to_string(),
                "flac".to_string(),
                "m4a".to_string(),
                "ogg".to_string(),
            ],
            auto_convert: true,
            conversion_timeout: 60,
            temp_file_cleanup: true,
            temp_file_retention: 300,
        }
    }
}

impl Default for WorkersConfig {
    fn default() -> Self {
        Self {
            transcription_workers: 3,
            channel_buffer_size: 100,
            worker_timeout: 3600,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            log_dir: "./logs".to_string(),
            max_file_size: "10MB".to_string(),
            max_files: 20,
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            pid_file: "./voice-cli.pid".to_string(),
            log_file: "./logs/daemon.log".to_string(),
            work_dir: "./".to_string(),
        }
    }
}

impl Default for TaskManagementConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 4,
            sqlite_db_path: "./data/tasks.db".to_string(),
            retry_attempts: 2,
            task_timeout_seconds: 3600,
            catch_panic: true,
            task_retention_minutes: 1440, // 24 hours in minutes
            sled_db_path: "./data/sled".to_string(),
        }
    }
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_text_length: default_tts_max_text_length(),
            supported_formats: default_tts_supported_formats(),
            engine: TtsEngineConfig::default(),
            streaming: TtsStreamingConfig::default(),
            tasks_db_path: default_tts_tasks_db_path(),
        }
    }
}

impl Default for TtsEngineConfig {
    fn default() -> Self {
        Self {
            pool_size: default_tts_pool_size(),
            device: default_tts_device(),
            default_model: default_tts_model(),
            default_sid: 0,
            default_speed: default_tts_speed(),
            default_length_scale: default_tts_length_scale(),
            default_language: default_tts_lang(),
            num_threads: default_tts_num_threads(),
            provider: None,
            models_dir: default_tts_models_dir(),
            debug: false,
        }
    }
}

impl Default for TtsStreamingConfig {
    fn default() -> Self {
        Self {
            idle_timeout_sec: default_tts_idle_timeout(),
            synth_timeout_sec: default_tts_synth_timeout(),
            default_format: default_tts_format(),
        }
    }
}

fn default_tts_max_text_length() -> usize {
    5000
}
fn default_tts_supported_formats() -> Vec<String> {
    vec!["wav".to_string(), "pcm".to_string()]
}
fn default_tts_pool_size() -> usize {
    1
}
fn default_tts_device() -> String {
    "cpu".to_string()
}
fn default_tts_model() -> String {
    "kokoro-multi-lang-v1_0".to_string()
}
fn default_tts_speed() -> f32 {
    1.0
}
fn default_tts_length_scale() -> f32 {
    1.0
}
fn default_tts_num_threads() -> i32 {
    4
}
fn default_tts_lang() -> Option<String> {
    // None：对齐官方 run-kokoro-zh-en.sh（不传 --kokoro-lang；lexicon 提供后 lang 非必需）。
    // 多语 Kokoro v1.0 由 lexicon + 文本自动判定语种。
    None
}
fn default_tts_models_dir() -> String {
    "./models/tts".to_string()
}
fn default_tts_idle_timeout() -> u64 {
    30
}
fn default_tts_synth_timeout() -> u64 {
    120
}
fn default_tts_format() -> String {
    "wav".to_string()
}
fn default_tts_tasks_db_path() -> String {
    "./data/tts_tasks.db".to_string()
}

/// 环境变量提供者抽象（依赖注入，避免直接读写全局 std::env）。
/// 生产用 [`StdEnv`]；测试用 [`MapEnv`] 注入，无全局副作用、可并行。
pub trait EnvProvider {
    fn get(&self, key: &str) -> Option<String>;
}

/// 生产实现：直接读 `std::env::var`
pub struct StdEnv;
impl EnvProvider for StdEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// 测试实现：基于 HashMap，零全局态
#[cfg(test)]
#[derive(Default)]
pub struct MapEnv(pub std::collections::HashMap<String, String>);
#[cfg(test)]
impl EnvProvider for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

impl Config {
    pub fn load(config_path: &PathBuf) -> crate::Result<Self> {
        let config_content = std::fs::read_to_string(config_path).map_err(|e| {
            crate::VoiceCliError::Config(format!(
                "Failed to read configuration file {:?}: {}",
                config_path, e
            ))
        })?;

        serde_yaml::from_str(&config_content).map_err(|e| {
            crate::VoiceCliError::Config(format!(
                "Failed to parse configuration file {:?}: {}",
                config_path, e
            ))
        })
    }

    /// Apply environment variable overrides to the configuration
    pub fn apply_env_overrides(&mut self, env: &dyn EnvProvider) -> crate::Result<()> {
        // Server configuration overrides
        if let Some(host) = env.get("VOICE_CLI_HOST") {
            if host.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_HOST environment variable cannot be empty".to_string(),
                ));
            }
            self.server.host = host.clone();
            tracing::info!("Applied environment override: VOICE_CLI_HOST = {}", host);
        }

        if let Some(port_str) = env.get("VOICE_CLI_PORT") {
            let port = port_str.parse::<u16>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_PORT value '{}': must be a valid port number (1-65535)",
                    port_str
                ))
            })?;
            self.server.port = port;
            tracing::info!("Applied environment override: VOICE_CLI_PORT = {}", port);
        }

        // Max file size override
        if let Some(size_str) = env.get("VOICE_CLI_MAX_FILE_SIZE") {
            let size = size_str.parse::<usize>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_MAX_FILE_SIZE value '{}': must be a valid number in bytes",
                    size_str
                ))
            })?;
            if size == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_MAX_FILE_SIZE must be greater than 0".to_string(),
                ));
            }
            self.server.max_file_size = size;
            tracing::info!(
                "Applied environment override: VOICE_CLI_MAX_FILE_SIZE = {}",
                size
            );
        }

        // CORS enabled override
        if let Some(cors_str) = env.get("VOICE_CLI_CORS_ENABLED") {
            let cors_enabled = cors_str.parse::<bool>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_CORS_ENABLED value '{}': must be 'true' or 'false'",
                    cors_str
                ))
            })?;
            self.server.cors_enabled = cors_enabled;
            tracing::info!(
                "Applied environment override: VOICE_CLI_CORS_ENABLED = {}",
                cors_enabled
            );
        }

        // Logging configuration overrides
        if let Some(level) = env.get("VOICE_CLI_LOG_LEVEL") {
            let level = level.to_lowercase();
            let valid_levels = ["trace", "debug", "info", "warn", "error"];
            if !valid_levels.contains(&level.as_str()) {
                return Err(crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_LOG_LEVEL value '{}': must be one of {:?}",
                    level, valid_levels
                )));
            }
            self.logging.level = level.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_LOG_LEVEL = {}",
                level
            );
        }

        if let Some(log_dir) = env.get("VOICE_CLI_LOG_DIR") {
            if log_dir.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_LOG_DIR environment variable cannot be empty".to_string(),
                ));
            }
            self.logging.log_dir = log_dir.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_LOG_DIR = {}",
                log_dir
            );
        }

        if let Some(max_files_str) = env.get("VOICE_CLI_LOG_MAX_FILES") {
            let max_files = max_files_str.parse::<u32>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_LOG_MAX_FILES value '{}': must be a valid number",
                    max_files_str
                ))
            })?;
            if max_files == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_LOG_MAX_FILES must be greater than 0".to_string(),
                ));
            }
            self.logging.max_files = max_files;
            tracing::info!(
                "Applied environment override: VOICE_CLI_LOG_MAX_FILES = {}",
                max_files
            );
        }

        // Whisper configuration overrides
        if let Some(model) = env.get("VOICE_CLI_DEFAULT_MODEL") {
            if model.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_DEFAULT_MODEL environment variable cannot be empty".to_string(),
                ));
            }
            self.whisper.default_model = model.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_DEFAULT_MODEL = {}",
                model
            );
        }

        if let Some(models_dir) = env.get("VOICE_CLI_MODELS_DIR") {
            if models_dir.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_MODELS_DIR environment variable cannot be empty".to_string(),
                ));
            }
            self.whisper.models_dir = models_dir.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_MODELS_DIR = {}",
                models_dir
            );
        }

        if let Some(auto_download_str) = env.get("VOICE_CLI_AUTO_DOWNLOAD") {
            let auto_download = auto_download_str.parse::<bool>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_AUTO_DOWNLOAD value '{}': must be 'true' or 'false'",
                    auto_download_str
                ))
            })?;
            self.whisper.auto_download = auto_download;
            tracing::info!(
                "Applied environment override: VOICE_CLI_AUTO_DOWNLOAD = {}",
                auto_download
            );
        }

        if let Some(workers_str) = env.get("VOICE_CLI_TRANSCRIPTION_WORKERS") {
            let workers = workers_str.parse::<usize>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_TRANSCRIPTION_WORKERS value '{}': must be a valid number",
                    workers_str
                ))
            })?;
            if workers == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_TRANSCRIPTION_WORKERS must be greater than 0".to_string(),
                ));
            }
            self.whisper.workers.transcription_workers = workers;
            tracing::info!(
                "Applied environment override: VOICE_CLI_TRANSCRIPTION_WORKERS = {}",
                workers
            );
        }

        // Daemon configuration overrides
        if let Some(work_dir) = env.get("VOICE_CLI_WORK_DIR") {
            if work_dir.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_WORK_DIR environment variable cannot be empty".to_string(),
                ));
            }
            self.daemon.work_dir = work_dir.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_WORK_DIR = {}",
                work_dir
            );
        }

        if let Some(pid_file) = env.get("VOICE_CLI_PID_FILE") {
            if pid_file.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_PID_FILE environment variable cannot be empty".to_string(),
                ));
            }
            self.daemon.pid_file = pid_file.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_PID_FILE = {}",
                pid_file
            );
        }

        if let Some(max_tasks_str) = env.get("VOICE_CLI_MAX_CONCURRENT_TASKS") {
            let max_tasks = max_tasks_str.parse::<usize>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_MAX_CONCURRENT_TASKS value '{}': must be a valid number",
                    max_tasks_str
                ))
            })?;
            if max_tasks == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_MAX_CONCURRENT_TASKS must be greater than 0".to_string(),
                ));
            }
            self.task_management.max_concurrent_tasks = max_tasks;
            tracing::info!(
                "Applied environment override: VOICE_CLI_MAX_CONCURRENT_TASKS = {}",
                max_tasks
            );
        }

        if let Some(db_path) = env.get("VOICE_CLI_SQLITE_DB_PATH") {
            if db_path.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_SQLITE_DB_PATH environment variable cannot be empty".to_string(),
                ));
            }
            self.task_management.sqlite_db_path = db_path.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_SQLITE_DB_PATH = {}",
                db_path
            );
        }

        if let Some(retention_minutes_str) = env.get("VOICE_CLI_TASK_RETENTION_MINUTES") {
            let retention_minutes = retention_minutes_str.parse::<u32>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_TASK_RETENTION_MINUTES value '{}': must be a valid number",
                    retention_minutes_str
                ))
            })?;
            if retention_minutes == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_TASK_RETENTION_MINUTES must be greater than 0".to_string(),
                ));
            }
            self.task_management.task_retention_minutes = retention_minutes;
            tracing::info!(
                "Applied environment override: VOICE_CLI_TASK_RETENTION_MINUTES = {}",
                retention_minutes
            );
        }

        if let Some(sled_path) = env.get("VOICE_CLI_SLED_DB_PATH") {
            if sled_path.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_SLED_DB_PATH environment variable cannot be empty".to_string(),
                ));
            }
            self.task_management.sled_db_path = sled_path.clone();
            tracing::info!(
                "Applied environment override: VOICE_CLI_SLED_DB_PATH = {}",
                sled_path
            );
        }

        Ok(())
    }

    pub fn save(&self, config_path: &PathBuf) -> crate::Result<()> {
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let config_yaml = serde_yaml::to_string(self)?;
        std::fs::write(config_path, config_yaml)?;
        Ok(())
    }

    pub fn models_dir_path(&self) -> PathBuf {
        PathBuf::from(&self.whisper.models_dir)
    }

    pub fn log_dir_path(&self) -> PathBuf {
        PathBuf::from(&self.logging.log_dir)
    }

    pub fn validate(&self) -> crate::Result<()> {
        // Validate server configuration
        if self.server.host.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "Server host cannot be empty".to_string(),
            ));
        }

        if self.server.port == 0 {
            return Err(crate::VoiceCliError::Config(
                "Server port must be between 1 and 65535".to_string(),
            ));
        }

        if self.server.max_file_size == 0 {
            return Err(crate::VoiceCliError::Config(
                "Max file size must be greater than 0".to_string(),
            ));
        }

        // Validate whisper configuration
        if self.whisper.default_model.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "Default model cannot be empty".to_string(),
            ));
        }

        if !self
            .whisper
            .supported_models
            .contains(&self.whisper.default_model)
        {
            return Err(crate::VoiceCliError::Config(format!(
                "Default model '{}' is not in supported models list",
                self.whisper.default_model
            )));
        }

        if self.whisper.models_dir.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "Models directory cannot be empty".to_string(),
            ));
        }

        if self.whisper.workers.transcription_workers == 0 {
            return Err(crate::VoiceCliError::Config(
                "Transcription workers must be greater than 0".to_string(),
            ));
        }

        // Validate streaming configuration（Fail Fast：decode_interval=0 会永不解码）
        let s = &self.whisper.streaming;
        if s.decode_interval_sec <= 0.0 {
            return Err(crate::VoiceCliError::Config(
                "streaming.decode_interval_sec must be > 0".to_string(),
            ));
        }
        if s.tail_trim_sec < 0.0 {
            return Err(crate::VoiceCliError::Config(
                "streaming.tail_trim_sec must be >= 0".to_string(),
            ));
        }
        if s.min_agree_count == 0 {
            return Err(crate::VoiceCliError::Config(
                "streaming.min_agree_count must be >= 1".to_string(),
            ));
        }

        // Validate logging configuration
        if self.logging.log_dir.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "Log directory cannot be empty".to_string(),
            ));
        }

        if self.logging.max_files == 0 {
            return Err(crate::VoiceCliError::Config(
                "Max log files must be greater than 0".to_string(),
            ));
        }

        let valid_log_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_log_levels.contains(&self.logging.level.to_lowercase().as_str()) {
            return Err(crate::VoiceCliError::Config(format!(
                "Invalid log level '{}'. Valid levels: {:?}",
                self.logging.level, valid_log_levels
            )));
        }

        // Validate daemon configuration
        if self.daemon.work_dir.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "Work directory cannot be empty".to_string(),
            ));
        }

        if self.daemon.pid_file.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "PID file path cannot be empty".to_string(),
            ));
        }

        // Validate task management configuration
        if self.task_management.max_concurrent_tasks == 0 {
            return Err(crate::VoiceCliError::Config(
                "Max concurrent tasks must be greater than 0".to_string(),
            ));
        }

        if self.task_management.sqlite_db_path.is_empty() {
            return Err(crate::VoiceCliError::Config(
                "SQLite database path cannot be empty".to_string(),
            ));
        }

        Ok(())
    }
}
