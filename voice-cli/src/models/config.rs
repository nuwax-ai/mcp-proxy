use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub whisper: WhisperConfig,
    pub logging: LoggingConfig,
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub cluster: ClusterConfig,
    #[serde(default)]
    pub load_balancer: LoadBalancerConfig,
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
pub struct ClusterConfig {
    /// Unique node identifier
    pub node_id: String,
    /// Address to bind gRPC server
    pub bind_address: String,
    /// Port for gRPC cluster communication
    pub grpc_port: u16,
    /// Port for HTTP API (same as server.port by default)
    pub http_port: u16,
    /// Whether this node can process tasks (true=leader can process, false=leader only coordinates)
    pub leader_can_process_tasks: bool,
    /// Heartbeat interval in seconds
    pub heartbeat_interval: u64,
    /// Election timeout in seconds
    pub election_timeout: u64,
    /// Path to store cluster metadata database
    pub metadata_db_path: String,
    /// Enable cluster mode (false for single-node operation)
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalancerConfig {
    /// Enable load balancer service
    pub enabled: bool,
    /// Address to bind load balancer
    pub bind_address: String,
    /// Port for load balancer service
    pub port: u16,
    /// Health check interval in seconds
    pub health_check_interval: u64,
    /// Health check timeout in seconds
    pub health_check_timeout: u64,
    /// PID file for load balancer daemon
    pub pid_file: String,
    /// Log file for load balancer
    pub log_file: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            whisper: WhisperConfig::default(),
            logging: LoggingConfig::default(),
            daemon: DaemonConfig::default(),
            cluster: ClusterConfig::default(),
            load_balancer: LoadBalancerConfig::default(),
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

impl Default for ClusterConfig {
    fn default() -> Self {
        use uuid::Uuid;
        Self {
            node_id: format!("node-{}", Uuid::new_v4().simple()),
            bind_address: "0.0.0.0".to_string(),
            grpc_port: 50051,
            http_port: 8080, // Same as server port by default
            leader_can_process_tasks: true, // Leader can process tasks by default
            heartbeat_interval: 3,
            election_timeout: 10,
            metadata_db_path: "./cluster_metadata".to_string(),
            enabled: false, // Disabled by default for backward compatibility
        }
    }
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            enabled: false, // Disabled by default
            bind_address: "0.0.0.0".to_string(),
            port: 8090, // Different port to avoid conflicts
            health_check_interval: 5,
            health_check_timeout: 3,
            pid_file: "./voice-cli-lb.pid".to_string(),
            log_file: "./logs/lb.log".to_string(),
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

        // Validate cluster configuration if enabled
        if self.cluster.enabled {
            if self.cluster.grpc_port == 0 {
                return Err(crate::VoiceCliError::Config("Invalid cluster gRPC port".to_string()));
            }
            
            if self.cluster.heartbeat_interval == 0 {
                return Err(crate::VoiceCliError::Config("Invalid heartbeat interval".to_string()));
            }
            
            if self.cluster.election_timeout == 0 {
                return Err(crate::VoiceCliError::Config("Invalid election timeout".to_string()));
            }
        }

        // Validate load balancer configuration if enabled
        if self.load_balancer.enabled {
            if self.load_balancer.port == 0 {
                return Err(crate::VoiceCliError::Config("Invalid load balancer port".to_string()));
            }
        }

        Ok(())
    }
    
    /// Get cluster metadata database path
    pub fn cluster_db_path(&self) -> PathBuf {
        PathBuf::from(&self.cluster.metadata_db_path)
    }
    
    /// Get load balancer PID file path
    pub fn lb_pid_file_path(&self) -> PathBuf {
        PathBuf::from(&self.load_balancer.pid_file)
    }
    
    /// Get load balancer log file path
    pub fn lb_log_file_path(&self) -> PathBuf {
        PathBuf::from(&self.load_balancer.log_file)
    }
}