use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};
use std::env;
use std::fs::File;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, OnceLock, RwLock};
use thiserror::Error;
use tracing::{info, warn};

/// 默认配置文件内容
const DEFAULT_CONFIG_YAML: &str = include_str!("../config.yml");

/// 配置验证错误
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("配置文件读取失败: {0}")]
    FileRead(String),
    #[error("配置解析失败: {0}")]
    Parse(String),
    #[error("配置验证失败: {field} - {message}")]
    Validation { field: String, message: String },
    #[error("环境变量解析失败: {var} - {message}")]
    EnvVar { var: String, message: String },
    #[error("路径无效: {path} - {message}")]
    InvalidPath { path: String, message: String },
}

/// 文件大小单位
#[derive(Debug, Clone)]
pub struct FileSize(pub u64);

impl<'de> Deserialize<'de> for FileSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_file_size(&s)
            .map(FileSize)
            .map_err(serde::de::Error::custom)
    }
}

impl FileSize {
    pub fn bytes(&self) -> u64 {
        self.0
    }

    pub fn mb(&self) -> u64 {
        self.0 / (1024 * 1024)
    }

    pub fn from_mb(mb: u64) -> Self {
        Self(mb * 1024 * 1024)
    }
}

/// 全局文件大小配置
#[derive(Debug, Clone, Deserialize)]
pub struct GlobalFileSizeConfig {
    /// 统一的最大文件大小限制
    pub max_file_size: FileSize,
    /// 大文档阈值（用于流式处理）
    pub large_document_threshold: FileSize,
}

impl Default for GlobalFileSizeConfig {
    fn default() -> Self {
        Self {
            max_file_size: FileSize(100 * 1024 * 1024),      // 100MB
            large_document_threshold: FileSize(1024 * 1024), // 1MB
        }
    }
}

impl GlobalFileSizeConfig {
    /// 创建新的全局文件大小配置实例
    pub fn new() -> Self {
        get_global_file_size_config().clone()
    }

    /// 验证配置的有效性
    pub fn validate(&self) -> Result<(), ConfigError> {
        let configs = [
            ("max_file_size", self.max_file_size.bytes()),
            (
                "large_document_threshold",
                self.large_document_threshold.bytes(),
            ),
        ];

        for (name, size) in configs {
            if size == 0 {
                return Err(ConfigError::Validation {
                    field: format!("file_size_config.{name}"),
                    message: "文件大小不能为0".to_string(),
                });
            }

            if size > 10 * 1024 * 1024 * 1024 {
                // 10GB
                return Err(ConfigError::Validation {
                    field: format!("file_size_config.{name}"),
                    message: "文件大小不能超过10GB".to_string(),
                });
            }
        }

        Ok(())
    }

    /// 获取指定用途的文件大小限制
    pub fn get_max_size_for(&self, _purpose: FileSizePurpose) -> u64 {
        // 统一使用同一个文件大小限制
        self.max_file_size.bytes()
    }

    /// 检查文件大小是否超过指定用途的限制
    pub fn is_size_allowed(&self, file_size: u64, purpose: FileSizePurpose) -> bool {
        file_size <= self.get_max_size_for(purpose)
    }

    /// 获取大文档阈值
    pub fn get_large_document_threshold(&self) -> u64 {
        self.large_document_threshold.bytes()
    }

    /// 检查是否为大文档
    pub fn is_large_document(&self, file_size: u64) -> bool {
        file_size >= self.get_large_document_threshold()
    }

    /// 获取指定用途的文件大小限制（返回FileSize结构体）
    pub fn get_file_size_limit(&self, _purpose: &FileSizePurpose) -> &FileSize {
        // 统一使用同一个文件大小限制
        &self.max_file_size
    }
}

/// 文件大小用途枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSizePurpose {
    /// 默认用途
    Default,
    /// 文档解析器
    DocumentParser,
    /// MinerU解析器
    MinerU,
    /// MarkItDown解析器
    MarkItDown,
    /// 图片处理器
    ImageProcessor,
    /// 文件上传
    Upload,
    /// 格式检测器
    FormatDetector,
    /// 内容验证
    ContentValidation,
    /// 缓存
    Cache,
}

/// 解析文件大小字符串 (例如: "100MB", "1GB", "500KB")
pub fn parse_file_size(size_str: &str) -> Result<u64, String> {
    let size_str = size_str.trim().to_uppercase();

    if let Some(pos) = size_str.find(|c: char| c.is_alphabetic()) {
        let (number_part, unit_part) = size_str.split_at(pos);
        let number: f64 = number_part
            .parse()
            .map_err(|_| format!("无效的数字: {number_part}"))?;

        let multiplier = match unit_part {
            "B" => 1,
            "KB" => 1024,
            "MB" => 1024 * 1024,
            "GB" => 1024 * 1024 * 1024,
            "TB" => 1024_u64.pow(4),
            _ => return Err(format!("不支持的单位: {unit_part}")),
        };

        Ok((number * multiplier as f64) as u64)
    } else {
        // 如果没有单位，假设是字节
        size_str
            .parse::<u64>()
            .map_err(|_| format!("无效的文件大小: {size_str}"))
    }
}

/// 配置构建器，用于测试和灵活配置创建
#[derive(Debug, Default)]
pub struct ConfigBuilder {
    environment: Option<String>,
    server: Option<ServerConfig>,
    log: Option<LogConfig>,
    document_parser: Option<DocumentParserConfig>,
    mineru: Option<MinerUConfig>,
    markitdown: Option<MarkItDownConfig>,
    storage: Option<StorageConfig>,
    external_integration: Option<ExternalIntegrationConfig>,
    file_size_config: Option<GlobalFileSizeConfig>,
}

impl ConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn environment(mut self, environment: String) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn server(mut self, server: ServerConfig) -> Self {
        self.server = Some(server);
        self
    }

    pub fn log(mut self, log: LogConfig) -> Self {
        self.log = Some(log);
        self
    }

    pub fn document_parser(mut self, document_parser: DocumentParserConfig) -> Self {
        self.document_parser = Some(document_parser);
        self
    }

    pub fn mineru(mut self, mineru: MinerUConfig) -> Self {
        self.mineru = Some(mineru);
        self
    }

    pub fn markitdown(mut self, markitdown: MarkItDownConfig) -> Self {
        self.markitdown = Some(markitdown);
        self
    }

    pub fn storage(mut self, storage: StorageConfig) -> Self {
        self.storage = Some(storage);
        self
    }

    pub fn external_integration(mut self, external_integration: ExternalIntegrationConfig) -> Self {
        self.external_integration = Some(external_integration);
        self
    }

    pub fn file_size_config(mut self, file_size_config: GlobalFileSizeConfig) -> Self {
        self.file_size_config = Some(file_size_config);
        self
    }

    pub fn build(self) -> Result<AppConfig, ConfigError> {
        // 使用默认配置作为基础
        let mut config: AppConfig = serde_yaml::from_str(DEFAULT_CONFIG_YAML)
            .map_err(|e| ConfigError::Parse(e.to_string()))?;

        // 应用构建器中的配置
        if let Some(environment) = self.environment {
            config.environment = environment;
        }
        if let Some(server) = self.server {
            config.server = server;
        }
        if let Some(log) = self.log {
            config.log = log;
        }
        if let Some(document_parser) = self.document_parser {
            config.document_parser = document_parser;
        }
        if let Some(mineru) = self.mineru {
            config.mineru = mineru;
        }
        if let Some(markitdown) = self.markitdown {
            config.markitdown = markitdown;
        }
        if let Some(storage) = self.storage {
            config.storage = storage;
        }
        if let Some(external_integration) = self.external_integration {
            config.external_integration = external_integration;
        }
        if let Some(file_size_config) = self.file_size_config {
            config.file_size_config = file_size_config;
        }

        // 验证配置
        config.validate()?;

        Ok(config)
    }
}

/// CUDA环境状态
#[derive(Debug, Clone, Default)]
pub struct CudaStatus {
    pub available: bool,
    pub version: Option<String>,
    pub device_count: usize,
    pub recommended_device: Option<String>,
}

/// 应用配置
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub environment: String,
    pub server: ServerConfig,
    pub log: LogConfig,
    pub document_parser: DocumentParserConfig,
    pub mineru: MinerUConfig,
    pub markitdown: MarkItDownConfig,
    pub storage: StorageConfig,
    pub external_integration: ExternalIntegrationConfig,
    /// 全局文件大小配置
    #[serde(default)]
    pub file_size_config: GlobalFileSizeConfig,
}

/// 服务器配置
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
    pub host: String,
}

impl ServerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.port == 0 {
            return Err(ConfigError::Validation {
                field: "server.port".to_string(),
                message: "端口号不能为0".to_string(),
            });
        }

        if self.host.is_empty() {
            return Err(ConfigError::Validation {
                field: "server.host".to_string(),
                message: "主机地址不能为空".to_string(),
            });
        }

        // 验证主机地址格式
        if !self
            .host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == ':' || c == '-')
        {
            return Err(ConfigError::Validation {
                field: "server.host".to_string(),
                message: "主机地址包含无效字符".to_string(),
            });
        }

        Ok(())
    }
}

/// 日志配置
#[derive(Debug, Clone, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub path: String,
    /// The number of log files to retain (default: 20)
    #[serde(default = "default_retain_days")]
    pub retain_days: u32,
}

/// Default log files to retain
fn default_retain_days() -> u32 {
    20
}

impl LogConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        // 验证日志级别
        let valid_levels = ["trace", "debug", "info", "warn", "error"];
        if !valid_levels.contains(&self.level.to_lowercase().as_str()) {
            return Err(ConfigError::Validation {
                field: "log.level".to_string(),
                message: format!(
                    "无效的日志级别: {}，支持的级别: {:?}",
                    self.level, valid_levels
                ),
            });
        }

        if self.path.is_empty() {
            return Err(ConfigError::Validation {
                field: "log.path".to_string(),
                message: "日志路径不能为空".to_string(),
            });
        }

        // 验证路径是否可以创建
        let path = Path::new(&self.path);
        if let Some(parent) = path.parent() {
            if parent.exists() && !parent.is_dir() {
                return Err(ConfigError::InvalidPath {
                    path: self.path.clone(),
                    message: "父目录不是一个有效的目录".to_string(),
                });
            }
        }

        Ok(())
    }
}

/// 文档解析配置
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentParserConfig {
    pub max_concurrent: usize,
    pub queue_size: usize,
    pub download_timeout: u32,
    pub processing_timeout: u32,
}

impl DocumentParserConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_concurrent == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.max_concurrent".to_string(),
                message: "最大并发数不能为0".to_string(),
            });
        }

        if self.max_concurrent > 100 {
            return Err(ConfigError::Validation {
                field: "document_parser.max_concurrent".to_string(),
                message: "最大并发数不能超过100".to_string(),
            });
        }

        if self.queue_size == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.queue_size".to_string(),
                message: "队列大小不能为0".to_string(),
            });
        }

        // 文件大小限制现在由全局配置管理

        if self.download_timeout == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.download_timeout".to_string(),
                message: "下载超时时间不能为0".to_string(),
            });
        }

        if self.processing_timeout == 0 {
            return Err(ConfigError::Validation {
                field: "document_parser.processing_timeout".to_string(),
                message: "处理超时时间不能为0".to_string(),
            });
        }

        Ok(())
    }
}

/// MinerU配置
#[derive(Debug, Clone, Deserialize)]
pub struct MinerUConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_python_path")]
    pub python_path: String,
    pub max_concurrent: usize,
    pub queue_size: usize,
    #[serde(default)]
    pub timeout: u32, // 0表示使用统一的processing_timeout
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default)]
    pub quality_level: QualityLevel,
    #[serde(default = "default_device")]
    pub device: String, // 推理设备：cpu/cuda/cuda:0/npu/mps等
    #[serde(default = "default_vram")]
    pub vram: u32, // 单进程最大GPU显存占用(GB)，仅对pipeline后端且支持CUDA时有效
}

/// 质量级别
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub enum QualityLevel {
    Fast,
    #[default]
    Balanced,
    HighQuality,
}

fn default_batch_size() -> usize {
    1
}

fn default_backend() -> String {
    "pipeline".to_string()
}

fn default_python_path() -> String {
    // Default to virtual environment python if available, otherwise system python
    if cfg!(windows) {
        "./venv/Scripts/python.exe".to_string()
    } else {
        "./venv/bin/python".to_string()
    }
}

fn default_device() -> String {
    "cpu".to_string()
}

fn default_vram() -> u32 {
    8 // 默认8GB显存限制
}

impl MinerUConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let valid_backends = [
            "pipeline",
            "vlm-transformers",
            "vlm-sglang-engine",
            "vlm-sglang-client",
        ];
        if !valid_backends.contains(&self.backend.as_str()) {
            return Err(ConfigError::Validation {
                field: "mineru.backend".to_string(),
                message: format!(
                    "无效的后端类型: {}，支持的类型: {:?}",
                    self.backend, valid_backends
                ),
            });
        }

        if self.python_path.is_empty() {
            return Err(ConfigError::Validation {
                field: "mineru.python_path".to_string(),
                message: "Python路径不能为空".to_string(),
            });
        }

        if self.max_concurrent == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.max_concurrent".to_string(),
                message: "最大并发数不能为0".to_string(),
            });
        }

        if self.queue_size == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.queue_size".to_string(),
                message: "队列大小不能为0".to_string(),
            });
        }

        // timeout 为 0 表示使用统一的 processing_timeout，这是允许的

        if self.batch_size == 0 {
            return Err(ConfigError::Validation {
                field: "mineru.batch_size".to_string(),
                message: "批处理大小不能为0".to_string(),
            });
        }

        Ok(())
    }

    /// Get the effective python path, auto-detecting virtual environment if needed
    pub fn get_effective_python_path(&self) -> String {
        // If the configured path is the default and a virtual environment exists, use it
        let default_path = default_python_path();
        if self.python_path == default_path
            || self.python_path == "python3"
            || self.python_path == "python"
        {
            let venv_python = if cfg!(windows) {
                std::path::Path::new("./venv/Scripts/python.exe")
            } else {
                std::path::Path::new("./venv/bin/python")
            };

            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }

        self.python_path.clone()
    }
}

/// MarkItDown配置
#[derive(Debug, Clone, Deserialize)]
pub struct MarkItDownConfig {
    #[serde(default = "default_python_path")]
    pub python_path: String,
    #[serde(default)]
    pub timeout: u32, // 0表示使用统一的processing_timeout
    pub enable_plugins: bool,
    pub features: MarkItDownFeatures,
}

impl MarkItDownConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.python_path.is_empty() {
            return Err(ConfigError::Validation {
                field: "markitdown.python_path".to_string(),
                message: "Python路径不能为空".to_string(),
            });
        }

        // timeout 为 0 表示使用统一的 processing_timeout，这是允许的

        Ok(())
    }

    /// Get the effective python path, auto-detecting virtual environment if needed
    pub fn get_effective_python_path(&self) -> String {
        // If the configured path is the default and a virtual environment exists, use it
        let default_path = default_python_path();
        if self.python_path == default_path
            || self.python_path == "python3"
            || self.python_path == "python"
        {
            let venv_python = if cfg!(windows) {
                std::path::Path::new("./venv/Scripts/python.exe")
            } else {
                std::path::Path::new("./venv/bin/python")
            };

            if venv_python.exists() {
                return venv_python.to_string_lossy().to_string();
            }
        }

        self.python_path.clone()
    }
}

/// MarkItDown功能配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkItDownFeatures {
    pub ocr: bool,
    pub audio_transcription: bool,
    pub azure_doc_intel: bool,
    pub youtube_transcription: bool,
}

/// 存储配置
#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    pub sled: SledConfig,
    pub oss: OssConfig,
}

impl StorageConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.sled.validate()?;
        self.oss.validate()?;
        Ok(())
    }
}

/// Sled数据库配置
#[derive(Debug, Clone, Deserialize)]
pub struct SledConfig {
    pub path: String,
    pub cache_capacity: usize,
}

impl SledConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.path.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.sled.path".to_string(),
                message: "数据库路径不能为空".to_string(),
            });
        }

        if self.cache_capacity == 0 {
            return Err(ConfigError::Validation {
                field: "storage.sled.cache_capacity".to_string(),
                message: "缓存容量不能为0".to_string(),
            });
        }

        Ok(())
    }
}

/// OSS配置
#[derive(Debug, Clone, Deserialize)]
pub struct OssConfig {
    pub endpoint: String,
    // pub bucket: String,
    /// 公有存储桶名称 (默认: nuwa-packages)
    pub public_bucket: String,
    /// 私有存储桶名称 (默认: edu-nuwa-packages)
    pub private_bucket: String,
    pub access_key_id: String,
    pub access_key_secret: String,
    /// 上传文件的统一子目录前缀
    #[serde(default = "default_upload_directory")]
    pub upload_directory: String,
    /// 区域 (默认: oss-rg-china-mainland)
    pub region: String,
}

/// 默认上传目录
fn default_upload_directory() -> String {
    "edu".to_string()
}

impl OssConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.endpoint.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.endpoint".to_string(),
                message: "OSS端点不能为空".to_string(),
            });
        }

        if self.public_bucket.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.public_bucket".to_string(),
                message: "OSS存储桶名称不能为空".to_string(),
            });
        }

        if self.private_bucket.is_empty() {
            return Err(ConfigError::Validation {
                field: "storage.oss.private_bucket".to_string(),
                message: "OSS存储桶名称不能为空".to_string(),
            });
        }
        // 注意：region 现在是可选的，可以为 None
        // 注意：access_key_id 和 access_key_secret 可以为空，因为它们可能通过环境变量设置

        Ok(())
    }

    /// 检查OSS配置是否完整（环境变量是否已设置）
    pub fn is_configured(&self) -> bool {
        !self.access_key_id.is_empty() && !self.access_key_secret.is_empty()
    }
}

/// 外部集成配置
#[derive(Debug, Clone, Deserialize)]
pub struct ExternalIntegrationConfig {
    pub webhook_url: String,
    pub api_key: String,
    pub timeout: u32,
}

impl ExternalIntegrationConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.timeout == 0 {
            return Err(ConfigError::Validation {
                field: "external_integration.timeout".to_string(),
                message: "超时时间不能为0".to_string(),
            });
        }

        // webhook_url 和 api_key 可以为空，因为外部集成是可选的
        if !self.webhook_url.is_empty() {
            // 简单的URL格式验证
            if !self.webhook_url.starts_with("http://") && !self.webhook_url.starts_with("https://")
            {
                return Err(ConfigError::Validation {
                    field: "external_integration.webhook_url".to_string(),
                    message: "Webhook URL必须以http://或https://开头".to_string(),
                });
            }
        }

        Ok(())
    }
}

impl AppConfig {
    /// 加载配置文件，支持多种配置源和环境变量覆盖
    pub fn load_config() -> Result<Self, ConfigError> {
        // 1. 首先加载基础配置
        let mut config = Self::load_base_config()?;

        // 2. 从环境变量覆盖配置
        config.load_all_from_env()?;

        // 3. 验证最终配置
        config.validate()?;

        // 4. 初始化必要的目录
        config.initialize_directories()?;

        Ok(config)
    }

    /// 加载基础配置文件
    pub fn load_base_config() -> Result<Self, ConfigError> {
        Self::load_base_config_with_path(None)
    }

    /// 加载基础配置文件，支持可选的配置文件路径
    pub fn load_base_config_with_path(config_path: Option<String>) -> Result<Self, ConfigError> {
        // 优先尝试从传入的路径加载
        if let Some(path) = config_path {
            if Path::new(&path).exists() {
                return Self::load_from_file(&path);
            }
        }

        let config_paths = [
            "/app/config.yml",
            "config.yml",
            "document-parser/config.yml",
        ];

        // 尝试从环境变量指定的配置文件路径加载
        if let Ok(env_config_path) = env::var("DOCUMENT_PARSER_CONFIG") {
            return Self::load_from_file(&env_config_path);
        }

        // 尝试从预定义路径加载
        for path in &config_paths {
            if Path::new(path).exists() {
                return Self::load_from_file(path);
            }
        }

        // 如果都没有找到，尝试在当前目录创建默认配置文件
        if let Err(e) = Self::create_default_config_in_current_dir() {
            warn!(
                "Unable to create default configuration file in current directory: {}",
                e
            );
        }

        // 使用默认配置
        serde_yaml::from_str::<AppConfig>(DEFAULT_CONFIG_YAML)
            .map_err(|e| ConfigError::Parse(format!("解析默认配置失败: {e}")))
    }

    /// 从指定文件加载配置
    fn load_from_file(path: &str) -> Result<Self, ConfigError> {
        let file = File::open(path)
            .map_err(|e| ConfigError::FileRead(format!("无法打开配置文件 {path}: {e}")))?;

        serde_yaml::from_reader(file)
            .map_err(|e| ConfigError::Parse(format!("解析配置文件 {path} 失败: {e}")))
    }

    /// 在当前目录创建默认配置文件
    fn create_default_config_in_current_dir() -> Result<(), ConfigError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| ConfigError::FileRead(format!("无法获取当前目录: {e}")))?;

        let config_path = current_dir.join("config.yml");

        // 如果配置文件已存在，不覆盖
        if config_path.exists() {
            return Ok(());
        }

        // 创建配置文件
        std::fs::write(&config_path, DEFAULT_CONFIG_YAML)
            .map_err(|e| ConfigError::FileRead(format!("无法创建默认配置文件: {e}")))?;

        info!(
            "The default configuration file has been created in the current directory: {}",
            config_path.display()
        );
        Ok(())
    }

    /// 验证整个配置的有效性
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.server.validate()?;
        self.log.validate()?;
        self.document_parser.validate()?;
        self.mineru.validate()?;
        self.markitdown.validate()?;
        self.storage.validate()?;
        self.external_integration.validate()?;
        self.file_size_config.validate()?;

        // 交叉验证
        self.cross_validate()?;

        Ok(())
    }

    /// 交叉验证不同配置项之间的一致性
    fn cross_validate(&self) -> Result<(), ConfigError> {
        // 验证并发配置的一致性
        if self.document_parser.max_concurrent < self.mineru.max_concurrent + 1 {
            return Err(ConfigError::Validation {
                field: "document_parser.max_concurrent".to_string(),
                message: "文档解析器的最大并发数应该大于MinerU的最大并发数".to_string(),
            });
        }

        // 验证队列大小的合理性
        if self.document_parser.queue_size < self.mineru.queue_size {
            return Err(ConfigError::Validation {
                field: "document_parser.queue_size".to_string(),
                message: "文档解析器的队列大小应该大于等于MinerU的队列大小".to_string(),
            });
        }

        // 验证超时配置的合理性（0表示使用统一超时，跳过验证）
        if self.mineru.timeout > 0 && self.document_parser.processing_timeout < self.mineru.timeout
        {
            return Err(ConfigError::Validation {
                field: "document_parser.processing_timeout".to_string(),
                message: "文档解析器的处理超时时间应该大于等于MinerU的超时时间".to_string(),
            });
        }

        if self.markitdown.timeout > 0
            && self.document_parser.processing_timeout < self.markitdown.timeout
        {
            return Err(ConfigError::Validation {
                field: "document_parser.processing_timeout".to_string(),
                message: "文档解析器的处理超时时间应该大于等于MarkItDown的超时时间".to_string(),
            });
        }

        Ok(())
    }

    /// 初始化必要的目录
    pub fn initialize_directories(&self) -> Result<(), ConfigError> {
        let directories = [&self.log.path, &self.storage.sled.path];

        for dir_path in &directories {
            let path = Path::new(dir_path);

            // 如果是文件路径，获取父目录
            let dir_to_create = if path.extension().is_some() {
                path.parent().unwrap_or(path)
            } else {
                path
            };

            if !dir_to_create.exists() {
                std::fs::create_dir_all(dir_to_create).map_err(|e| ConfigError::InvalidPath {
                    path: dir_to_create.to_string_lossy().to_string(),
                    message: format!("无法创建目录: {e}"),
                })?;
            }
        }

        Ok(())
    }

    /// 从环境变量加载所有配置，支持类型安全的解析和错误处理
    pub fn load_all_from_env(&mut self) -> Result<(), ConfigError> {
        self.load_server_config_from_env()?;
        self.load_log_config_from_env()?;
        self.load_document_parser_config_from_env()?;
        self.load_oss_config_from_env()?;
        self.load_mineru_config_from_env()?;
        self.load_markitdown_config_from_env()?;
        self.load_external_integration_config_from_env()?;
        Ok(())
    }

    /// 从环境变量加载服务器配置
    fn load_server_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(port_str) = env::var("SERVER_PORT") {
            self.server.port = Self::parse_env_var("SERVER_PORT", &port_str)?;
        }
        if let Ok(host) = env::var("SERVER_HOST") {
            self.server.host = host;
        }
        Ok(())
    }

    /// 从环境变量加载日志配置
    fn load_log_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(level) = env::var("LOG_LEVEL") {
            self.log.level = level;
        }
        if let Ok(path) = env::var("LOG_PATH") {
            self.log.path = path;
        }
        Ok(())
    }

    /// 从环境变量加载文档解析器配置
    fn load_document_parser_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(max_concurrent_str) = env::var("DOCUMENT_PARSER_MAX_CONCURRENT") {
            self.document_parser.max_concurrent =
                Self::parse_env_var("DOCUMENT_PARSER_MAX_CONCURRENT", &max_concurrent_str)?;
        }
        if let Ok(queue_size_str) = env::var("DOCUMENT_PARSER_QUEUE_SIZE") {
            self.document_parser.queue_size =
                Self::parse_env_var("DOCUMENT_PARSER_QUEUE_SIZE", &queue_size_str)?;
        }
        // 文件大小限制现在由全局配置管理
        if let Ok(download_timeout_str) = env::var("DOCUMENT_PARSER_DOWNLOAD_TIMEOUT") {
            self.document_parser.download_timeout =
                Self::parse_env_var("DOCUMENT_PARSER_DOWNLOAD_TIMEOUT", &download_timeout_str)?;
        }
        if let Ok(processing_timeout_str) = env::var("DOCUMENT_PARSER_PROCESSING_TIMEOUT") {
            self.document_parser.processing_timeout = Self::parse_env_var(
                "DOCUMENT_PARSER_PROCESSING_TIMEOUT",
                &processing_timeout_str,
            )?;
        }
        Ok(())
    }

    /// 从环境变量加载OSS配置
    fn load_oss_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(endpoint) = env::var("ALIYUN_OSS_ENDPOINT") {
            self.storage.oss.endpoint = endpoint;
        }
        if let Ok(public_bucket) = env::var("ALIYUN_OSS_PUBLIC_BUCKET") {
            self.storage.oss.public_bucket = public_bucket;
        }
        if let Ok(private_bucket) = env::var("ALIYUN_OSS_PRIVATE_BUCKET") {
            self.storage.oss.private_bucket = private_bucket;
        }
        if let Ok(access_key_id) = env::var("OSS_ACCESS_KEY_ID") {
            self.storage.oss.access_key_id = access_key_id;
        }
        if let Ok(access_key_secret) = env::var("OSS_ACCESS_KEY_SECRET") {
            self.storage.oss.access_key_secret = access_key_secret;
        }
        Ok(())
    }

    /// 从环境变量加载MinerU配置
    fn load_mineru_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(backend) = env::var("MINERU_BACKEND") {
            self.mineru.backend = backend;
        }
        if let Ok(python_path) = env::var("MINERU_PYTHON_PATH") {
            self.mineru.python_path = python_path;
        }
        if let Ok(max_concurrent_str) = env::var("MINERU_MAX_CONCURRENT") {
            self.mineru.max_concurrent =
                Self::parse_env_var("MINERU_MAX_CONCURRENT", &max_concurrent_str)?;
        }
        if let Ok(queue_size_str) = env::var("MINERU_QUEUE_SIZE") {
            self.mineru.queue_size = Self::parse_env_var("MINERU_QUEUE_SIZE", &queue_size_str)?;
        }
        if let Ok(timeout_str) = env::var("MINERU_TIMEOUT") {
            self.mineru.timeout = Self::parse_env_var("MINERU_TIMEOUT", &timeout_str)?;
        }
        if let Ok(batch_size_str) = env::var("MINERU_BATCH_SIZE") {
            self.mineru.batch_size = Self::parse_env_var("MINERU_BATCH_SIZE", &batch_size_str)?;
        }
        if let Ok(device) = env::var("MINERU_DEVICE") {
            self.mineru.device = device;
        }
        Ok(())
    }

    /// 从环境变量加载MarkItDown配置
    fn load_markitdown_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(python_path) = env::var("MARKITDOWN_PYTHON_PATH") {
            self.markitdown.python_path = python_path;
        }
        if let Ok(timeout_str) = env::var("MARKITDOWN_TIMEOUT") {
            self.markitdown.timeout = Self::parse_env_var("MARKITDOWN_TIMEOUT", &timeout_str)?;
        }
        if let Ok(enable_plugins_str) = env::var("MARKITDOWN_ENABLE_PLUGINS") {
            self.markitdown.enable_plugins =
                Self::parse_env_var("MARKITDOWN_ENABLE_PLUGINS", &enable_plugins_str)?;
        }
        if let Ok(enable_ocr_str) = env::var("MARKITDOWN_ENABLE_OCR") {
            self.markitdown.features.ocr =
                Self::parse_env_var("MARKITDOWN_ENABLE_OCR", &enable_ocr_str)?;
        }
        if let Ok(enable_audio_transcription_str) =
            env::var("MARKITDOWN_ENABLE_AUDIO_TRANSCRIPTION")
        {
            self.markitdown.features.audio_transcription = Self::parse_env_var(
                "MARKITDOWN_ENABLE_AUDIO_TRANSCRIPTION",
                &enable_audio_transcription_str,
            )?;
        }
        if let Ok(enable_azure_doc_intel_str) = env::var("MARKITDOWN_ENABLE_AZURE_DOC_INTEL") {
            self.markitdown.features.azure_doc_intel = Self::parse_env_var(
                "MARKITDOWN_ENABLE_AZURE_DOC_INTEL",
                &enable_azure_doc_intel_str,
            )?;
        }
        if let Ok(enable_youtube_transcription_str) =
            env::var("MARKITDOWN_ENABLE_YOUTUBE_TRANSCRIPTION")
        {
            self.markitdown.features.youtube_transcription = Self::parse_env_var(
                "MARKITDOWN_ENABLE_YOUTUBE_TRANSCRIPTION",
                &enable_youtube_transcription_str,
            )?;
        }
        Ok(())
    }

    /// 从环境变量加载外部集成配置
    fn load_external_integration_config_from_env(&mut self) -> Result<(), ConfigError> {
        if let Ok(webhook_url) = env::var("EXTERNAL_INTEGRATION_WEBHOOK_URL") {
            self.external_integration.webhook_url = webhook_url;
        }
        if let Ok(api_key) = env::var("EXTERNAL_INTEGRATION_API_KEY") {
            self.external_integration.api_key = api_key;
        }
        if let Ok(timeout_str) = env::var("EXTERNAL_INTEGRATION_TIMEOUT") {
            self.external_integration.timeout =
                Self::parse_env_var("EXTERNAL_INTEGRATION_TIMEOUT", &timeout_str)?;
        }
        Ok(())
    }

    /// 类型安全的环境变量解析
    fn parse_env_var<T>(var_name: &str, value: &str) -> Result<T, ConfigError>
    where
        T: FromStr,
        T::Err: std::fmt::Display,
    {
        value.parse::<T>().map_err(|e| ConfigError::EnvVar {
            var: var_name.to_string(),
            message: format!("无法解析值 '{value}': {e}"),
        })
    }

    /// 获取配置构建器，用于测试
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::new()
    }

    /// 生成配置摘要，用于日志记录（隐藏敏感信息）
    pub fn summary(&self) -> String {
        format!(
            "AppConfig {{ server: {}:{}, log: {}, max_concurrent: {}, storage: sled={}, oss={}:{} }}",
            self.server.host,
            self.server.port,
            self.log.level,
            self.document_parser.max_concurrent,
            self.storage.sled.path,
            self.storage.oss.endpoint,
            self.storage.oss.public_bucket
        )
    }
}

/// 全局配置实例
static GLOBAL_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// 初始化全局配置
pub fn init_global_config(config: AppConfig) -> Result<(), ConfigError> {
    // 检查是否已经初始化
    if GLOBAL_CONFIG.get().is_some() {
        return Ok(()); // 已经初始化过了，直接返回成功
    }

    GLOBAL_CONFIG
        .set(config)
        .map_err(|_| ConfigError::Validation {
            field: "global_config".to_string(),
            message: "全局配置已经初始化过了".to_string(),
        })?;
    Ok(())
}

/// 获取全局配置引用
pub fn get_global_config() -> &'static AppConfig {
    GLOBAL_CONFIG
        .get()
        .expect("全局配置尚未初始化，请先调用 init_global_config")
}

/// 获取全局文件大小配置
pub fn get_global_file_size_config() -> &'static GlobalFileSizeConfig {
    let max_file_sieze = &get_global_config().file_size_config;
    info!("Global file size configuration: {:?}", max_file_sieze);
    max_file_sieze
}

/// 便捷函数：获取指定用途的文件大小限制
pub fn get_file_size_limit(purpose: &FileSizePurpose) -> &'static FileSize {
    get_global_file_size_config().get_file_size_limit(purpose)
}

/// 便捷函数：检查文件大小是否允许
pub fn is_file_size_allowed(file_size: u64, purpose: FileSizePurpose) -> bool {
    get_global_file_size_config().is_size_allowed(file_size, purpose)
}

/// 便捷函数：检查是否为大文档
pub fn is_large_document(file_size: u64) -> bool {
    get_global_file_size_config().is_large_document(file_size)
}

/// 便捷函数：获取大文档阈值
pub fn get_large_document_threshold() -> u64 {
    get_global_file_size_config().get_large_document_threshold()
}

/// 便捷函数：检查CUDA是否可用
pub fn is_cuda_available() -> bool {
    get_global_cuda_status_clone().available
}

/// 便捷函数：获取推荐的CUDA设备
pub fn get_recommended_cuda_device() -> Option<String> {
    get_global_cuda_status_clone().recommended_device
}

/// 全局CUDA状态管理，使用线程安全的Arc<RwLock<>>
static GLOBAL_CUDA_STATUS: OnceLock<Arc<RwLock<CudaStatus>>> = OnceLock::new();

/// 初始化全局CUDA状态
pub fn init_global_cuda_status(cuda_status: CudaStatus) -> Result<(), ConfigError> {
    GLOBAL_CUDA_STATUS
        .set(Arc::new(RwLock::new(cuda_status)))
        .map_err(|_| ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "全局CUDA状态已经初始化过了".to_string(),
        })?;
    Ok(())
}

/// 更新全局CUDA状态
pub fn update_global_cuda_status(cuda_status: CudaStatus) -> Result<(), ConfigError> {
    if let Some(global_status) = GLOBAL_CUDA_STATUS.get() {
        let mut status = global_status.write().map_err(|_| ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "无法获取CUDA状态写锁".to_string(),
        })?;
        *status = cuda_status;
        Ok(())
    } else {
        Err(ConfigError::Validation {
            field: "global_cuda_status".to_string(),
            message: "全局CUDA状态尚未初始化".to_string(),
        })
    }
}

/// 获取全局CUDA状态的克隆
pub fn get_global_cuda_status_clone() -> CudaStatus {
    if let Some(global_status) = GLOBAL_CUDA_STATUS.get() {
        if let Ok(status) = global_status.read() {
            return status.clone();
        }
    }
    warn!("The global CUDA state has not been initialized and returns to the default value.");
    CudaStatus::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::env;
    use tempfile::TempDir;

    #[test]
    fn test_file_size_parsing() {
        assert_eq!(parse_file_size("100B").unwrap(), 100);
        assert_eq!(parse_file_size("1KB").unwrap(), 1024);
        assert_eq!(parse_file_size("1MB").unwrap(), 1024 * 1024);
        assert_eq!(parse_file_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_file_size("2.5MB").unwrap(),
            (2.5 * 1024.0 * 1024.0) as u64
        );

        // 测试无效格式
        assert!(parse_file_size("invalid").is_err());
        assert!(parse_file_size("100XB").is_err());
        assert!(parse_file_size("").is_err());
    }

    #[test]
    fn test_default_config_loading() {
        let config = AppConfig::load_base_config().unwrap();

        // 验证默认值
        assert_eq!(config.server.port, 8087);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.log.level, "info");
        assert_eq!(config.document_parser.max_concurrent, 5); // 配置文件中的实际值
    }

    #[test]
    fn test_config_validation() {
        let mut config = AppConfig::load_base_config().unwrap();

        // 测试有效配置
        assert!(config.validate().is_ok());

        // 测试无效端口
        config.server.port = 0;
        assert!(config.validate().is_err());

        // 恢复有效端口，测试无效日志级别
        config.server.port = 8087;
        config.log.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_builder() {
        let server_config = ServerConfig {
            port: 9000,
            host: "127.0.0.1".to_string(),
        };

        let config = ConfigBuilder::new().server(server_config).build().unwrap();

        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");
    }

    #[test]
    #[ignore = "Modifies global environment variables, causes race conditions with parallel tests"]
    fn test_environment_variable_override() {
        // 设置环境变量
        unsafe {
            env::set_var("SERVER_PORT", "9999");
            env::set_var("LOG_LEVEL", "debug");
        }

        let mut config = AppConfig::load_base_config().unwrap();
        config.load_all_from_env().unwrap();

        assert_eq!(config.server.port, 9999);
        assert_eq!(config.log.level, "debug");

        // 清理环境变量
        unsafe {
            env::remove_var("SERVER_PORT");
            env::remove_var("LOG_LEVEL");
        }
    }

    #[test]
    #[ignore = "Modifies global environment variables, causes race conditions with parallel tests"]
    fn test_invalid_environment_variables() {
        // 设置无效的环境变量
        unsafe {
            env::set_var("SERVER_PORT", "invalid_port");
        }

        let mut config = AppConfig::load_base_config().unwrap();
        let result = config.load_all_from_env();

        assert!(result.is_err());

        // 清理环境变量
        unsafe {
            env::remove_var("SERVER_PORT");
        }
    }

    #[test]
    fn test_cross_validation() {
        let mut config = AppConfig::load_base_config().unwrap();

        // 设置不一致的并发配置
        config.document_parser.max_concurrent = 1;
        config.mineru.max_concurrent = 5;

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_directory_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path().to_string_lossy().to_string();

        let mut config = AppConfig::load_base_config().unwrap();
        config.log.path = format!("{temp_path}/logs/app.log");
        // temp_dir is now hardcoded, no need to set it
        config.storage.sled.path = format!("{temp_path}/sled");

        assert!(config.initialize_directories().is_ok());

        // 验证目录是否创建
        assert!(Path::new(&format!("{temp_path}/logs")).exists());
        // 注意：initialize_directories 方法不再创建 temp/mineru 和 temp/markitdown
        assert!(Path::new(&format!("{temp_path}/sled")).exists());
    }

    #[test]
    fn test_server_config_validation() {
        let mut config = ServerConfig {
            port: 8080,
            host: "localhost".to_string(),
        };

        assert!(config.validate().is_ok());

        // 测试无效端口
        config.port = 0;
        assert!(config.validate().is_err());

        // 测试空主机
        config.port = 8080;
        config.host = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_log_config_validation() {
        let mut config = LogConfig {
            level: "info".to_string(),
            path: "/tmp/test.log".to_string(),
            retain_days: 20,
        };

        assert!(config.validate().is_ok());

        // 测试无效日志级别
        config.level = "invalid".to_string();
        assert!(config.validate().is_err());

        // 测试空路径
        config.level = "info".to_string();
        config.path = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_document_parser_config_validation() {
        let config = DocumentParserConfig {
            max_concurrent: 3,
            queue_size: 100,
            download_timeout: 3600,
            processing_timeout: 1800,
        };

        assert!(config.validate().is_ok());

        // 测试零并发
        let mut invalid_config = config.clone();
        invalid_config.max_concurrent = 0;
        assert!(invalid_config.validate().is_err());

        // 测试过大的并发数
        invalid_config.max_concurrent = 200;
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_mineru_config_validation() {
        let config = MinerUConfig {
            backend: "pipeline".to_string(),

            python_path: "/usr/bin/python3".to_string(),
            max_concurrent: 3,
            queue_size: 100,
            timeout: 0, // 使用统一超时配置
            batch_size: 10,
            quality_level: QualityLevel::Balanced,
            device: "cpu".to_string(),
            vram: 8, // 默认显存限制
        };

        assert!(config.validate().is_ok());

        // 测试无效后端
        let mut invalid_config = config.clone();
        invalid_config.backend = "invalid".to_string();
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_external_integration_config_validation() {
        let config = ExternalIntegrationConfig {
            webhook_url: "https://example.com/webhook".to_string(),
            api_key: "test-key".to_string(),
            timeout: 30,
        };

        assert!(config.validate().is_ok());

        // 测试无效URL
        let mut invalid_config = config.clone();
        invalid_config.webhook_url = "invalid-url".to_string();
        assert!(invalid_config.validate().is_err());

        // 测试零超时
        invalid_config.webhook_url = "https://example.com/webhook".to_string();
        invalid_config.timeout = 0;
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_config_summary() {
        let config = AppConfig::load_base_config().unwrap();
        let summary = config.summary();

        assert!(summary.contains("AppConfig"));
        assert!(summary.contains("0.0.0.0:8087"));
        assert!(summary.contains("info"));
        assert!(!summary.contains("access_key")); // 确保敏感信息不在摘要中
    }
}
