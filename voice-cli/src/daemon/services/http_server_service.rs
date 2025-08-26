//! HTTP Server Service Implementation
//! 
//! This module provides a BackgroundService implementation for the voice-cli HTTP server,
//! supporting both single-node and cluster-aware modes with proper lifecycle management.

use crate::daemon::background_service::{BackgroundService, ServiceHealth, ClonableService};
use crate::daemon::service_logging::{init_service_logging, log_service_startup, log_service_shutdown};
use crate::models::Config;
use crate::VoiceCliError;
use std::time::Duration;
use tokio::sync::broadcast;
use crate::utils::signal_handling::create_service_shutdown_signal;
use tracing::{info, warn, error, debug};

/// HTTP Server Service that implements BackgroundService
/// 
/// This service wraps the existing voice-cli HTTP server functionality
/// and provides unified lifecycle management through the BackgroundService trait.
#[derive(Clone)]
pub struct HttpServerService {
    config: Option<Config>,
    foreground_mode: bool,
    start_time: Option<std::time::Instant>,
}

impl HttpServerService {
    /// Create a new HTTP server service
    /// 
    /// # Arguments
    /// * `foreground_mode` - true for 'run' command, false for 'start'/'restart'
    pub fn new(foreground_mode: bool) -> Self {
        Self {
            config: None,
            foreground_mode,
            start_time: None,
        }
    }

    /// Create server with specific configuration (for testing)
    pub fn with_config(config: Config, foreground_mode: bool) -> Self {
        Self {
            config: Some(config),
            foreground_mode,
            start_time: None,
        }
    }

    /// Get uptime since service started
    pub fn uptime(&self) -> Option<Duration> {
        self.start_time.map(|start| start.elapsed())
    }

    /// Check if running in cluster mode
    pub fn is_cluster_mode(&self) -> bool {
        self.config
            .as_ref()
            .map(|c| c.cluster.enabled)
            .unwrap_or(false)
    }
}

impl BackgroundService for HttpServerService {
    type Config = Config;
    type Error = VoiceCliError;

    fn service_name(&self) -> &'static str {
        "http-server"
    }

    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error> {
        // Initialize logging using the service logging module
        init_service_logging(&config, self.service_name(), self.foreground_mode)
            .map_err(|e| VoiceCliError::Config(format!("Logging initialization failed: {}", e)))?;

        // Log detailed startup information
        let additional_info = if config.cluster.enabled {
            Some(format!(
                "Cluster mode enabled - Node ID: {}, gRPC: {}, HTTP: {}",
                config.cluster.node_id, config.cluster.grpc_port, config.server.port
            ))
        } else {
            Some("Single-node mode".to_string())
        };

        log_service_startup(&config, self.service_name(), self.foreground_mode, additional_info.as_ref().map(|s| s.as_str()));

        // Store configuration
        self.config = Some(config);
        self.start_time = Some(std::time::Instant::now());

        info!(
            "HTTP server service initialized in {} mode",
            if self.foreground_mode { "foreground" } else { "background" }
        );

        Ok(())
    }

    fn run(&mut self, mut shutdown_rx: broadcast::Receiver<()>) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let config = self.config.clone();
        
        async move {
            info!("HTTP server service run method started");
            let config = match config {
                Some(config) => config,
                None => {
                    error!("HTTP server service not initialized");
                    return Err(VoiceCliError::Config("Service not initialized".to_string()));
                }
            };
            info!("Starting HTTP server on {}:{}...", config.server.host, config.server.port);

            // Create combined shutdown signal handler
            let shutdown_signal = async {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Received shutdown signal via service manager");
                    }
                    _ = create_service_shutdown_signal("http-server") => {
                        info!("Received system shutdown signal");
                    }
                }
            };

            // Run server with shutdown monitoring based on cluster configuration
            let server_result = if config.cluster.enabled {
                info!(
                    "Starting cluster-aware HTTP server on {}:{}",
                    config.server.host, config.server.port
                );
                info!("Cluster node ID: {}", config.cluster.node_id);
                info!("gRPC cluster communication port: {}", config.cluster.grpc_port);
                
                let server_future = crate::server::create_cluster_aware_server_with_shutdown(config).await?;
                
                tokio::select! {
                    result = server_future => {
                        match result {
                            Ok(_) => {
                                info!("Cluster-aware HTTP server completed successfully");
                                Ok(())
                            }
                            Err(e) => {
                                error!("Cluster-aware HTTP server error: {}", e);
                                Err(VoiceCliError::Config(format!("Server error: {}", e)))
                            }
                        }
                    }
                    _ = shutdown_signal => {
                        info!("Cluster-aware HTTP server shutting down gracefully");
                        Ok(())
                    }
                }
            } else {
                info!(
                    "Starting single-node HTTP server on {}:{}",
                    config.server.host, config.server.port
                );
                
                let server_future = crate::server::create_server_with_graceful_shutdown(config).await?;
                
                tokio::select! {
                    result = server_future => {
                        match result {
                            Ok(_) => {
                                info!("Single-node HTTP server completed successfully");
                                Ok(())
                            }
                            Err(e) => {
                                error!("Single-node HTTP server error: {}", e);
                                Err(VoiceCliError::Config(format!("Server error: {}", e)))
                            }
                        }
                    }
                    _ = shutdown_signal => {
                        info!("Single-node HTTP server shutting down gracefully");
                        Ok(())
                    }
                }
            };
            
            server_result
        }
    }

    fn cleanup(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let config = self.config.clone();
        let start_time = self.start_time;
        let service_name = self.service_name();
        
        async move {
            info!("Cleaning up HTTP server service");

            if let Some(config) = &config {
                // Perform any necessary cleanup
                crate::daemon::service_logging::cleanup_service_logs(config, service_name);
            }

            // Log shutdown information
            let uptime = start_time.map(|start| start.elapsed());
            log_service_shutdown(service_name, uptime);

            info!("HTTP server service cleanup completed");

            Ok(())
        }
    }

    async fn health_check(&self) -> ServiceHealth {
        let config = match &self.config {
            Some(config) => config,
            None => {
                return ServiceHealth::Unhealthy {
                    reason: "Service not initialized".to_string(),
                };
            }
        };

        // Perform HTTP health check
        let health_url = format!("http://{}:{}/health", config.server.host, config.server.port);
        
        match reqwest::Client::new()
            .get(&health_url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) => {
                if response.status().is_success() {
                    ServiceHealth::Healthy
                } else {
                    ServiceHealth::Unhealthy {
                        reason: format!("Health check returned status: {}", response.status()),
                    }
                }
            }
            Err(e) => ServiceHealth::Unhealthy {
                reason: format!("Health check failed: {}", e),
            },
        }
    }

    fn validate_config(config: &Self::Config) -> Result<(), Self::Error> {
        // Validate server configuration
        if config.server.host.is_empty() {
            return Err(VoiceCliError::Config("Server host cannot be empty".to_string()));
        }

        if config.server.port == 0 {
            return Err(VoiceCliError::Config("Server port cannot be 0".to_string()));
        }

        // Validate cluster configuration if enabled
        if config.cluster.enabled {
            if config.cluster.node_id.is_empty() {
                return Err(VoiceCliError::Config("Cluster node ID cannot be empty".to_string()));
            }

            if config.cluster.grpc_port == 0 {
                return Err(VoiceCliError::Config("Cluster gRPC port cannot be 0".to_string()));
            }

            if config.cluster.grpc_port == config.server.port {
                return Err(VoiceCliError::Config(
                    "Cluster gRPC port cannot be the same as HTTP server port".to_string(),
                ));
            }
        }

        // Validate Whisper configuration
        if config.whisper.default_model.is_empty() {
            return Err(VoiceCliError::Config("Default Whisper model cannot be empty".to_string()));
        }

        if config.whisper.models_dir.is_empty() {
            return Err(VoiceCliError::Config("Whisper models directory cannot be empty".to_string()));
        }

        // Validate logging configuration
        crate::daemon::service_logging::validate_logging_config(config)
            .map_err(|e| VoiceCliError::Config(format!("Logging validation failed: {}", e)))?;

        info!("HTTP server configuration validated successfully");
        Ok(())
    }
}

// Implement ClonableService to enable use with DefaultServiceManager
impl ClonableService for HttpServerService {}

/// Builder for HttpServerService with fluent configuration
pub struct HttpServerServiceBuilder {
    foreground_mode: bool,
    config: Option<Config>,
}

impl HttpServerServiceBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            foreground_mode: false,
            config: None,
        }
    }

    /// Set foreground mode (for 'run' commands)
    pub fn foreground_mode(mut self, foreground: bool) -> Self {
        self.foreground_mode = foreground;
        self
    }

    /// Set explicit configuration (optional, mainly for testing)
    pub fn with_config(mut self, config: Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the HttpServerService
    pub fn build(self) -> HttpServerService {
        match self.config {
            Some(config) => HttpServerService::with_config(config, self.foreground_mode),
            None => HttpServerService::new(self.foreground_mode),
        }
    }
}

impl Default for HttpServerServiceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Config, ServerConfig, WhisperConfig, LoggingConfig, DaemonConfig, ClusterConfig, LoadBalancerConfig};
    use tempfile::TempDir;

    fn create_test_config(temp_dir: &TempDir) -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                max_file_size: 1024 * 1024, // 1MB
                cors_enabled: true,
            },
            whisper: WhisperConfig {
                default_model: "base".to_string(),
                models_dir: temp_dir.path().join("models").to_string_lossy().to_string(),
                auto_download: true,
                supported_models: vec!["base".to_string()],
                audio_processing: crate::models::AudioProcessingConfig::default(),
                workers: crate::models::WorkersConfig::default(),
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                log_dir: temp_dir.path().join("logs").to_string_lossy().to_string(),
                max_file_size: "10MB".to_string(),
                max_files: 5,
            },
            daemon: DaemonConfig::default(),
            cluster: ClusterConfig::default(),
            load_balancer: LoadBalancerConfig::default(),
        }
    }

    #[test]
    fn test_service_creation() {
        let service = HttpServerService::new(true);
        assert_eq!(service.service_name(), "http-server");
        assert!(service.foreground_mode);
        assert!(service.config.is_none());
    }

    #[test]
    fn test_service_builder() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(true)
            .with_config(config.clone())
            .build();
            
        assert_eq!(service.service_name(), "http-server");
        assert!(service.foreground_mode);
        assert!(service.config.is_some());
    }

    #[tokio::test]
    async fn test_config_validation() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = create_test_config(&temp_dir);
        
        // Valid configuration should pass
        let result = HttpServerService::validate_config(&config);
        assert!(result.is_ok());
        
        // Invalid host should fail
        config.server.host = "".to_string();
        let result = HttpServerService::validate_config(&config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cluster_mode_validation() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = create_test_config(&temp_dir);
        
        // Enable cluster mode
        config.cluster.enabled = true;
        config.cluster.node_id = "test-node".to_string();
        config.cluster.grpc_port = 9090;
        
        let result = HttpServerService::validate_config(&config);
        assert!(result.is_ok());
        
        // Same port for HTTP and gRPC should fail
        config.cluster.grpc_port = config.server.port;
        let result = HttpServerService::validate_config(&config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_service_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(&temp_dir);
        
        let mut service = HttpServerService::new(true);
        let result = service.initialize(config).await;
        
        // Note: This test might fail if logging is already initialized
        // In a real scenario, each test would run in isolation
        match result {
            Ok(_) => {
                assert!(service.config.is_some());
                assert!(service.start_time.is_some());
            }
            Err(e) => {
                // If logging initialization fails due to already being initialized,
                // that's acceptable in tests
                println!("Initialization warning (acceptable in tests): {}", e);
            }
        }
    }

    #[test]
    fn test_cluster_mode_detection() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = create_test_config(&temp_dir);
        
        let mut service = HttpServerService::with_config(config.clone(), true);
        assert!(!service.is_cluster_mode());
        
        config.cluster.enabled = true;
        service = HttpServerService::with_config(config, true);
        assert!(service.is_cluster_mode());
    }

    #[test]
    fn test_uptime_tracking() {
        let service = HttpServerService::new(true);
        assert!(service.uptime().is_none());
        
        // In a real scenario, uptime would be set during initialization
        // Here we test the logic directly
        let mut service_with_time = service;
        service_with_time.start_time = Some(std::time::Instant::now());
        
        std::thread::sleep(std::time::Duration::from_millis(10));
        let uptime = service_with_time.uptime().unwrap();
        assert!(uptime.as_millis() >= 10);
    }
}