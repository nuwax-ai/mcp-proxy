use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 监听地址
    #[serde(default = "default_host")]
    pub host: String,

    /// 监听端口
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

/// FastEmbed 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FastEmbedConfig {
    /// 缓存目录
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    /// 默认模型
    #[serde(default = "default_model")]
    pub default_model: String,

    /// 批处理大小
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_cache_dir() -> String {
    ".fastembed_cache".to_string()
}

fn default_model() -> String {
    "BGELargeZHV15".to_string()
}

fn default_batch_size() -> usize {
    256
}

impl Default for FastEmbedConfig {
    fn default() -> Self {
        Self {
            cache_dir: default_cache_dir(),
            default_model: default_model(),
            batch_size: default_batch_size(),
        }
    }
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub fastembed: FastEmbedConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            fastembed: FastEmbedConfig::default(),
        }
    }
}

impl AppConfig {
    /// 从文件加载配置
    pub fn from_file(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("无法读取配置文件: {:?}", path))?;

        let config: AppConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("无法解析配置文件: {:?}", path))?;

        Ok(config)
    }

    /// 生成默认配置文件
    pub fn generate_default_config(path: &PathBuf) -> Result<()> {
        let default_config = AppConfig::default();
        let yaml = serde_yaml::to_string(&default_config).context("无法序列化默认配置")?;

        std::fs::write(path, yaml).with_context(|| format!("无法写入配置文件: {:?}", path))?;

        tracing::info!("Default configuration file has been generated: {:?}", path);
        Ok(())
    }

    /// 应用环境变量覆盖
    pub fn apply_env_overrides(&mut self) {
        // FASTEMBED_CACHE_DIR 可以覆盖 cache_dir
        if let Ok(cache_dir) = std::env::var("FASTEMBED_CACHE_DIR") {
            tracing::info!("Environment variable FASTEMBED_CACHE_DIR overrides the cache directory: {}", cache_dir);
            self.fastembed.cache_dir = cache_dir;
        }
    }

    /// 加载或生成配置
    pub fn load_or_generate(config_path: Option<PathBuf>) -> Result<Self> {
        let path = config_path.unwrap_or_else(|| PathBuf::from("./config.yml"));

        let mut config = if path.exists() {
            tracing::info!("Load configuration from file: {:?}", path);
            Self::from_file(&path)?
        } else {
            tracing::warn!("Configuration file does not exist: {:?}, generate default configuration", path);
            Self::generate_default_config(&path)?;
            Self::default()
        };

        // 应用环境变量覆盖
        config.apply_env_overrides();

        // 打印最终配置
        tracing::info!("Final configuration: {:?}", config);

        Ok(config)
    }
}
