use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub whisper: WhisperConfig,
    pub logging: LoggingConfig,
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub task_management: TaskManagementConfig,
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
    pub audio_processing: AudioProcessingConfig,
    pub workers: WorkersConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessingConfig {
    pub supported_formats: Vec<String>,
    pub auto_convert: bool,
    pub conversion_timeout: u32,
    pub temp_file_cleanup: bool,
    pub temp_file_retention: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkersConfig {
    pub transcription_workers: usize,
    pub channel_buffer_size: usize,
    pub worker_timeout: u32,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagementConfig {
    pub enabled: bool,
    pub max_concurrent_tasks: usize,
    pub sqlite_db_path: String,
    pub retry_attempts: usize,
    pub task_timeout_seconds: u64,
    pub catch_panic: bool,
    pub task_retention_days: u32,
    pub sled_db_path: String,
}


impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            whisper: WhisperConfig::default(),
            logging: LoggingConfig::default(),
            daemon: DaemonConfig::default(),
            task_management: TaskManagementConfig::default(),
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
            audio_processing: AudioProcessingConfig::default(),
            workers: WorkersConfig::default(),
        }
    }
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

impl Default for TaskManagementConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent_tasks: 4,
            sqlite_db_path: "./data/tasks.db".to_string(),
            retry_attempts: 3,
            task_timeout_seconds: 3600,
            catch_panic: true,
            task_retention_days: 1,
            sled_db_path: "./data/sled".to_string(),
        }
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

    pub fn load_or_create(config_path: &PathBuf) -> crate::Result<Self> {
        Self::load_with_env_overrides(config_path)
    }

    /// Load configuration with environment variable overrides
    pub fn load_with_env_overrides(config_path: &PathBuf) -> crate::Result<Self> {
        let mut config = if config_path.exists() {
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
            })?
        } else {
            let default_config = Config::default();
            default_config.save(config_path).map_err(|e| {
                crate::VoiceCliError::Config(format!(
                    "Failed to create default configuration file {:?}: {}",
                    config_path, e
                ))
            })?;
            tracing::info!("Created default configuration file at {:?}", config_path);
            default_config
        };

        // Apply environment variable overrides
        config.apply_env_overrides()?;

        // Validate the final configuration
        config.validate()?;

        Ok(config)
    }

    /// Apply environment variable overrides to the configuration
    pub fn apply_env_overrides(&mut self) -> crate::Result<()> {
        // Server configuration overrides
        if let Ok(host) = std::env::var("VOICE_CLI_HOST") {
            if host.trim().is_empty() {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_HOST environment variable cannot be empty".to_string(),
                ));
            }
            self.server.host = host.clone();
            tracing::info!("Applied environment override: VOICE_CLI_HOST = {}", host);
        }

        if let Ok(port_str) = std::env::var("VOICE_CLI_PORT") {
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
        if let Ok(size_str) = std::env::var("VOICE_CLI_MAX_FILE_SIZE") {
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
        if let Ok(cors_str) = std::env::var("VOICE_CLI_CORS_ENABLED") {
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
        if let Ok(level) = std::env::var("VOICE_CLI_LOG_LEVEL") {
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

        if let Ok(log_dir) = std::env::var("VOICE_CLI_LOG_DIR") {
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

        if let Ok(max_files_str) = std::env::var("VOICE_CLI_LOG_MAX_FILES") {
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
        if let Ok(model) = std::env::var("VOICE_CLI_DEFAULT_MODEL") {
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

        if let Ok(models_dir) = std::env::var("VOICE_CLI_MODELS_DIR") {
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

        if let Ok(auto_download_str) = std::env::var("VOICE_CLI_AUTO_DOWNLOAD") {
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

        if let Ok(workers_str) = std::env::var("VOICE_CLI_TRANSCRIPTION_WORKERS") {
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
        if let Ok(work_dir) = std::env::var("VOICE_CLI_WORK_DIR") {
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

        if let Ok(pid_file) = std::env::var("VOICE_CLI_PID_FILE") {
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

        // Task management configuration overrides
        if let Ok(enabled_str) = std::env::var("VOICE_CLI_TASK_MANAGEMENT_ENABLED") {
            let enabled = enabled_str.parse::<bool>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_TASK_MANAGEMENT_ENABLED value '{}': must be 'true' or 'false'",
                    enabled_str
                ))
            })?;
            self.task_management.enabled = enabled;
            tracing::info!(
                "Applied environment override: VOICE_CLI_TASK_MANAGEMENT_ENABLED = {}",
                enabled
            );
        }

        if let Ok(max_tasks_str) = std::env::var("VOICE_CLI_MAX_CONCURRENT_TASKS") {
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

        if let Ok(db_path) = std::env::var("VOICE_CLI_SQLITE_DB_PATH") {
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

        if let Ok(retention_days_str) = std::env::var("VOICE_CLI_TASK_RETENTION_DAYS") {
            let retention_days = retention_days_str.parse::<u32>().map_err(|_| {
                crate::VoiceCliError::Config(format!(
                    "Invalid VOICE_CLI_TASK_RETENTION_DAYS value '{}': must be a valid number",
                    retention_days_str
                ))
            })?;
            if retention_days == 0 {
                return Err(crate::VoiceCliError::Config(
                    "VOICE_CLI_TASK_RETENTION_DAYS must be greater than 0".to_string(),
                ));
            }
            self.task_management.task_retention_days = retention_days;
            tracing::info!(
                "Applied environment override: VOICE_CLI_TASK_RETENTION_DAYS = {}",
                retention_days
            );
        }

        if let Ok(sled_path) = std::env::var("VOICE_CLI_SLED_DB_PATH") {
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
        if self.task_management.enabled {
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
        }


        Ok(())
    }

}
