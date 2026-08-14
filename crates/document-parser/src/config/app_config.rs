//! 应用根配置 AppConfig：多源加载（文件/环境变量注入）、交叉验证、目录初始化、
//! 环境变量提供者抽象。

use super::*;

/// CUDA环境状态
#[derive(Debug, Clone, Default)]
pub struct CudaStatus {
    pub available: bool,
    pub version: Option<String>,
    pub device_count: usize,
    pub recommended_device: Option<String>,
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// 环境变量提供者抽象（依赖注入，避免直接读写全局 std::env）。
/// 生产用 [`StdEnv`] 读真实环境；测试用 [`MapEnv`] 注入，无全局副作用、可并行。
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            environment: "development".to_string(),
            server: ServerConfig::default(),
            log: LogConfig::default(),
            document_parser: DocumentParserConfig::default(),
            mineru: MinerUConfig::default(),
            markitdown: MarkItDownConfig::default(),
            storage: StorageConfig::default(),
            external_integration: ExternalIntegrationConfig::default(),
            file_size_config: GlobalFileSizeConfig::default(),
        }
    }
}

impl AppConfig {
    /// 加载配置文件，支持多种配置源和环境变量覆盖
    pub fn load_config() -> Result<Self, ConfigError> {
        // 1. 首先加载基础配置
        let mut config = Self::load_base_config()?;

        // 2. 从环境变量覆盖配置
        config.load_all_from_env(&StdEnv)?;

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
        if let Some(path) = config_path
            && Path::new(&path).exists()
        {
            return Self::load_from_file(&path);
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

        // 使用代码默认配置
        Ok(Self::default())
    }

    /// 从指定文件加载配置
    fn load_from_file(path: &str) -> Result<Self, ConfigError> {
        let file = File::open(path)
            .map_err(|e| ConfigError::FileRead(format!("无法打开配置文件 {path}: {e}")))?;

        serde_yaml::from_reader(file)
            .map_err(|e| ConfigError::Parse(format!("解析配置文件 {path} 失败: {e}")))
    }

    /// 将 `AppConfig::default()` 序列化为 YAML 写入指定路径（不覆盖已存在文件）。
    pub fn write_default_config(path: &Path) -> Result<(), ConfigError> {
        if path.exists() {
            return Ok(());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| {
                ConfigError::FileRead(format!("无法创建配置目录 {}: {e}", parent.display()))
            })?;
        }
        let yaml = serde_yaml::to_string(&Self::default())
            .map_err(|e| ConfigError::Parse(format!("序列化默认配置失败: {e}")))?;
        std::fs::write(path, yaml).map_err(|e| {
            ConfigError::FileRead(format!("无法写入默认配置文件 {}: {e}", path.display()))
        })?;
        info!(
            "Default configuration written from code defaults: {}",
            path.display()
        );
        Ok(())
    }

    /// 在当前目录创建默认配置文件（来自 `AppConfig::default()`）
    fn create_default_config_in_current_dir() -> Result<(), ConfigError> {
        let current_dir = std::env::current_dir()
            .map_err(|e| ConfigError::FileRead(format!("无法获取当前目录: {e}")))?;
        let config_path = current_dir.join("config.yml");
        Self::write_default_config(&config_path)
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

    /// 从环境变量加载所有配置，支持类型安全的解析和错误处理。
    /// `env` 注入环境变量来源：生产传 [`StdEnv`]，测试传 [`MapEnv`]。
    pub fn load_all_from_env(&mut self, env: &dyn EnvProvider) -> Result<(), ConfigError> {
        self.load_server_config_from_env(env)?;
        self.load_log_config_from_env(env)?;
        self.load_document_parser_config_from_env(env)?;
        self.load_oss_config_from_env(env)?;
        self.load_mineru_config_from_env(env)?;
        self.load_markitdown_config_from_env(env)?;
        self.load_external_integration_config_from_env(env)?;
        Ok(())
    }

    /// 从环境变量加载服务器配置
    fn load_server_config_from_env(&mut self, env: &dyn EnvProvider) -> Result<(), ConfigError> {
        if let Some(port_str) = env.get("SERVER_PORT") {
            self.server.port = Self::parse_env_var("SERVER_PORT", &port_str)?;
        }
        if let Some(host) = env.get("SERVER_HOST") {
            self.server.host = host;
        }
        Ok(())
    }

    /// 从环境变量加载日志配置
    fn load_log_config_from_env(&mut self, env: &dyn EnvProvider) -> Result<(), ConfigError> {
        if let Some(level) = env.get("LOG_LEVEL") {
            self.log.level = level;
        }
        if let Some(path) = env.get("LOG_PATH") {
            self.log.path = path;
        }
        Ok(())
    }

    /// 从环境变量加载文档解析器配置
    fn load_document_parser_config_from_env(
        &mut self,
        env: &dyn EnvProvider,
    ) -> Result<(), ConfigError> {
        if let Some(max_concurrent_str) = env.get("DOCUMENT_PARSER_MAX_CONCURRENT") {
            self.document_parser.max_concurrent =
                Self::parse_env_var("DOCUMENT_PARSER_MAX_CONCURRENT", &max_concurrent_str)?;
        }
        if let Some(queue_size_str) = env.get("DOCUMENT_PARSER_QUEUE_SIZE") {
            self.document_parser.queue_size =
                Self::parse_env_var("DOCUMENT_PARSER_QUEUE_SIZE", &queue_size_str)?;
        }
        // 文件大小限制现在由全局配置管理
        if let Some(download_timeout_str) = env.get("DOCUMENT_PARSER_DOWNLOAD_TIMEOUT") {
            self.document_parser.download_timeout =
                Self::parse_env_var("DOCUMENT_PARSER_DOWNLOAD_TIMEOUT", &download_timeout_str)?;
        }
        if let Some(processing_timeout_str) = env.get("DOCUMENT_PARSER_PROCESSING_TIMEOUT") {
            self.document_parser.processing_timeout = Self::parse_env_var(
                "DOCUMENT_PARSER_PROCESSING_TIMEOUT",
                &processing_timeout_str,
            )?;
        }
        if let Some(sync_size_str) = env.get("DOCUMENT_PARSER_SYNC_PARSE_MAX_FILE_SIZE") {
            self.document_parser.sync_parse_max_file_size = FileSize(
                parse_file_size(&sync_size_str).map_err(|e| ConfigError::EnvVar {
                    var: "DOCUMENT_PARSER_SYNC_PARSE_MAX_FILE_SIZE".to_string(),
                    message: e,
                })?,
            );
        }
        if let Some(sync_concurrent_str) = env.get("DOCUMENT_PARSER_SYNC_PARSE_MAX_CONCURRENT") {
            self.document_parser.sync_parse_max_concurrent = Self::parse_env_var(
                "DOCUMENT_PARSER_SYNC_PARSE_MAX_CONCURRENT",
                &sync_concurrent_str,
            )?;
        }
        if let Some(sync_timeout_str) = env.get("DOCUMENT_PARSER_SYNC_PARSE_TIMEOUT_SECS") {
            self.document_parser.sync_parse_timeout_secs =
                Self::parse_env_var("DOCUMENT_PARSER_SYNC_PARSE_TIMEOUT_SECS", &sync_timeout_str)?;
        }
        Ok(())
    }

    /// 从环境变量加载OSS配置
    fn load_oss_config_from_env(&mut self, env: &dyn EnvProvider) -> Result<(), ConfigError> {
        if let Some(endpoint) = env.get("ALIYUN_OSS_ENDPOINT") {
            self.storage.oss.endpoint = endpoint;
        }
        if let Some(public_bucket) = env.get("ALIYUN_OSS_PUBLIC_BUCKET") {
            self.storage.oss.public_bucket = public_bucket;
        }
        if let Some(private_bucket) = env.get("ALIYUN_OSS_PRIVATE_BUCKET") {
            self.storage.oss.private_bucket = private_bucket;
        }
        if let Some(access_key_id) = env.get("OSS_ACCESS_KEY_ID") {
            self.storage.oss.access_key_id = access_key_id;
        }
        if let Some(access_key_secret) = env.get("OSS_ACCESS_KEY_SECRET") {
            self.storage.oss.access_key_secret = access_key_secret;
        }
        Ok(())
    }

    /// 从环境变量加载MinerU配置
    fn load_mineru_config_from_env(&mut self, env: &dyn EnvProvider) -> Result<(), ConfigError> {
        if let Some(backend) = env.get("MINERU_BACKEND") {
            self.mineru.backend = backend;
        }
        if let Some(python_path) = env.get("MINERU_PYTHON_PATH") {
            self.mineru.python_path = python_path;
        }
        if let Some(max_concurrent_str) = env.get("MINERU_MAX_CONCURRENT") {
            self.mineru.max_concurrent =
                Self::parse_env_var("MINERU_MAX_CONCURRENT", &max_concurrent_str)?;
        }
        if let Some(queue_size_str) = env.get("MINERU_QUEUE_SIZE") {
            self.mineru.queue_size = Self::parse_env_var("MINERU_QUEUE_SIZE", &queue_size_str)?;
        }
        if let Some(timeout_str) = env.get("MINERU_TIMEOUT") {
            self.mineru.timeout = Self::parse_env_var("MINERU_TIMEOUT", &timeout_str)?;
        }
        if let Some(batch_size_str) = env.get("MINERU_BATCH_SIZE") {
            self.mineru.batch_size = Self::parse_env_var("MINERU_BATCH_SIZE", &batch_size_str)?;
        }
        if let Some(device) = env.get("MINERU_DEVICE") {
            self.mineru.device = device;
        }
        Ok(())
    }

    /// 从环境变量加载MarkItDown配置
    fn load_markitdown_config_from_env(
        &mut self,
        env: &dyn EnvProvider,
    ) -> Result<(), ConfigError> {
        if let Some(python_path) = env.get("MARKITDOWN_PYTHON_PATH") {
            self.markitdown.python_path = python_path;
        }
        if let Some(timeout_str) = env.get("MARKITDOWN_TIMEOUT") {
            self.markitdown.timeout = Self::parse_env_var("MARKITDOWN_TIMEOUT", &timeout_str)?;
        }
        if let Some(enable_plugins_str) = env.get("MARKITDOWN_ENABLE_PLUGINS") {
            self.markitdown.enable_plugins =
                Self::parse_env_var("MARKITDOWN_ENABLE_PLUGINS", &enable_plugins_str)?;
        }
        if let Some(enable_ocr_str) = env.get("MARKITDOWN_ENABLE_OCR") {
            self.markitdown.features.ocr =
                Self::parse_env_var("MARKITDOWN_ENABLE_OCR", &enable_ocr_str)?;
        }
        if let Some(enable_audio_transcription_str) =
            env.get("MARKITDOWN_ENABLE_AUDIO_TRANSCRIPTION")
        {
            self.markitdown.features.audio_transcription = Self::parse_env_var(
                "MARKITDOWN_ENABLE_AUDIO_TRANSCRIPTION",
                &enable_audio_transcription_str,
            )?;
        }
        if let Some(enable_azure_doc_intel_str) = env.get("MARKITDOWN_ENABLE_AZURE_DOC_INTEL") {
            self.markitdown.features.azure_doc_intel = Self::parse_env_var(
                "MARKITDOWN_ENABLE_AZURE_DOC_INTEL",
                &enable_azure_doc_intel_str,
            )?;
        }
        if let Some(enable_youtube_transcription_str) =
            env.get("MARKITDOWN_ENABLE_YOUTUBE_TRANSCRIPTION")
        {
            self.markitdown.features.youtube_transcription = Self::parse_env_var(
                "MARKITDOWN_ENABLE_YOUTUBE_TRANSCRIPTION",
                &enable_youtube_transcription_str,
            )?;
        }
        Ok(())
    }

    /// 从环境变量加载外部集成配置
    fn load_external_integration_config_from_env(
        &mut self,
        env: &dyn EnvProvider,
    ) -> Result<(), ConfigError> {
        if let Some(webhook_url) = env.get("EXTERNAL_INTEGRATION_WEBHOOK_URL") {
            self.external_integration.webhook_url = webhook_url;
        }
        if let Some(api_key) = env.get("EXTERNAL_INTEGRATION_API_KEY") {
            self.external_integration.api_key = api_key;
        }
        if let Some(timeout_str) = env.get("EXTERNAL_INTEGRATION_TIMEOUT") {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config_loading() {
        let config = AppConfig::load_base_config().unwrap();

        // 验证默认值
        assert_eq!(config.server.port, 8077);
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
        config.server.port = 8077;
        config.log.level = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_environment_variable_override() {
        // 通过 EnvProvider 注入，不碰全局 env，可并行
        let env = MapEnv(
            [
                ("SERVER_PORT".to_string(), "9999".to_string()),
                ("LOG_LEVEL".to_string(), "debug".to_string()),
            ]
            .into_iter()
            .collect(),
        );

        let mut config = AppConfig::load_base_config().unwrap();
        config.load_all_from_env(&env).unwrap();

        assert_eq!(config.server.port, 9999);
        assert_eq!(config.log.level, "debug");
    }

    #[test]
    fn test_invalid_environment_variables() {
        // 通过 EnvProvider 注入无效值，不碰全局 env
        let env = MapEnv(
            [("SERVER_PORT".to_string(), "invalid_port".to_string())]
                .into_iter()
                .collect(),
        );

        let mut config = AppConfig::load_base_config().unwrap();
        let result = config.load_all_from_env(&env);

        assert!(result.is_err());
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
    fn test_config_summary() {
        let config = AppConfig::load_base_config().unwrap();
        let summary = config.summary();

        assert!(summary.contains("AppConfig"));
        assert!(summary.contains("0.0.0.0:8077"));
        assert!(summary.contains("info"));
        assert!(!summary.contains("access_key")); // 确保敏感信息不在摘要中
    }
}
