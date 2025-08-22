use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub whisper: WhisperConfig,
    pub logging: LoggingConfig,
    pub daemon: DaemonConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub max_file_size: usize,
    pub cors_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperConfig {
    pub default_model: String,
    pub models_dir: String,
    pub auto_download: bool,
    pub supported_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub log_dir: String,
    pub max_file_size: String,
    pub max_files: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub pid_file: String,
    pub log_file: String,
    pub work_dir: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            whisper: WhisperConfig::default(),
            logging: LoggingConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
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
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            log_dir: "./logs".to_string(),
            max_file_size: "10MB".to_string(),
            max_files: 5,
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

impl Config {
    pub fn load_or_create(config_path: &PathBuf) -> crate::Result<Self> {
        if config_path.exists() {
            let config_content = std::fs::read_to_string(config_path)?;
            let config: Config = serde_yaml::from_str(&config_content)?;
            Ok(config)
        } else {
            let default_config = Config::default();
            default_config.save(config_path)?;
            tracing::info!("Created default configuration file at {:?}", config_path);
            Ok(default_config)
        }
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
        if self.server.port == 0 {
            return Err(crate::VoiceCliError::Config("Invalid port number".to_string()));
        }

        if self.server.max_file_size == 0 {
            return Err(crate::VoiceCliError::Config("Invalid max file size".to_string()));
        }

        if !self.whisper.supported_models.contains(&self.whisper.default_model) {
            return Err(crate::VoiceCliError::Config(
                format!("Default model '{}' is not in supported models list", self.whisper.default_model)
            ));
        }

        Ok(())
    }
}