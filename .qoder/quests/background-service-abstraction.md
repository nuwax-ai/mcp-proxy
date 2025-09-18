# Background Service Abstraction Design

## Overview

This design provides a unified background service abstraction for voice-cli that standardizes daemon implementations across different commands (`voice-cli server start`, `voice-cli cluster start`, etc.) following modern Rust best practices.

**Requirements:**
- Rust 1.85+ (for native async traits)
- Cargo.toml with `edition = "2024"`

**Core Principles:**
- **In-Process Architecture**: All services run within the same process using Tokio
- **Memory Safety**: Zero unsafe code, leveraging Rust's ownership system
- **Graceful Lifecycle**: Proper startup, shutdown, and restart semantics
- **Unified Interface**: Consistent API across different service types
- **Command Modes**: `run` for foreground testing, `start`/`restart` for background daemon
- **Centralized Logging**: All commands log to `./logs/` directory relative to voice-cli binary

## Current Implementation Analysis

### Problems with Existing Code

1. **Code Duplication**: Similar lifecycle logic across multiple services
2. **Inconsistent APIs**: Different interfaces for start/stop/restart
3. **Manual Resource Management**: Complex handle and channel cleanup
4. **Process Spawning**: Some services use `std::process::Command` (problematic)

The current `SafeDaemonService` and `ModernDaemonService` already show good patterns but lack unification.

## Core Service Abstraction

### Service Trait

```rust
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Unified background service trait (Rust 1.85 + edition 2024)
pub trait BackgroundService: Send + Sync + 'static {
    type Config: Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Service identifier for logging
    fn service_name(&self) -> &'static str;

    /// Initialize service with configuration
    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error>;

    /// Run the main service loop (cancellation-aware)
    async fn run(&mut self, shutdown_rx: broadcast::Receiver<()>) -> Result<(), Self::Error>;

    /// Perform cleanup before shutdown
    async fn cleanup(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Health check implementation
    async fn health_check(&self) -> ServiceHealth;

    /// Configuration validation
    fn validate_config(config: &Self::Config) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceHealth {
    Healthy,
    Unhealthy { reason: String },
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed { error: String },
}
```

### Universal Service Manager

```rust
/// Universal service manager for all background services
pub struct ServiceManager<S: BackgroundService> {
    service: S,
    config: S::Config,
    status: Arc<parking_lot::RwLock<ServiceStatus>>,
    shutdown_tx: Option<broadcast::Sender<()>>,
    service_handle: Option<JoinHandle<Result<(), S::Error>>>,
    start_time: Option<std::time::Instant>,
}

impl<S: BackgroundService> ServiceManager<S> {
    pub fn new(service: S, config: S::Config) -> Self {
        Self {
            service,
            config,
            status: Arc::new(parking_lot::RwLock::new(ServiceStatus::Stopped)),
            shutdown_tx: None,
            service_handle: None,
            start_time: None,
        }
    }

    /// Start the service in background
    pub async fn start(&mut self) -> Result<(), ServiceError> {
        if self.is_running() {
            return Err(ServiceError::AlreadyRunning(self.service.service_name()));
        }

        info!("Starting {} service", self.service.service_name());
        *self.status.write() = ServiceStatus::Starting;

        // Validate configuration
        S::validate_config(&self.config)
            .map_err(|e| ServiceError::ConfigurationError(e.to_string()))?;

        // Initialize service
        self.service.initialize(self.config.clone()).await
            .map_err(|e| ServiceError::InitializationFailed(e.to_string()))?;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone for background task
        let mut service_clone = self.service.clone();
        let service_name = self.service.service_name();
        let status_clone = self.status.clone();

        // Spawn service task
        let service_handle = tokio::spawn(async move {
            *status_clone.write() = ServiceStatus::Running;
            
            let result = service_clone.run(shutdown_rx).await;
            
            // Cleanup regardless of result
            if let Err(cleanup_error) = service_clone.cleanup().await {
                warn!("Cleanup error for {}: {}", service_name, cleanup_error);
            }
            
            match &result {
                Ok(_) => {
                    info!("{} service completed successfully", service_name);
                    *status_clone.write() = ServiceStatus::Stopped;
                }
                Err(e) => {
                    error!("{} service failed: {}", service_name, e);
                    *status_clone.write() = ServiceStatus::Failed { 
                        error: e.to_string() 
                    };
                }
            }
            
            result
        });

        self.service_handle = Some(service_handle);
        self.start_time = Some(std::time::Instant::now());

        info!("{} service started successfully", self.service.service_name());
        Ok(())
    }

    /// Stop the service gracefully
    pub async fn stop(&mut self) -> Result<(), ServiceError> {
        if !self.is_running() {
            return Ok(());
        }

        info!("Stopping {} service", self.service.service_name());
        *self.status.write() = ServiceStatus::Stopping;

        // Send shutdown signal
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Wait for service to stop with timeout
        if let Some(service_handle) = self.service_handle.take() {
            match tokio::time::timeout(Duration::from_secs(30), service_handle).await {
                Ok(Ok(Ok(_))) => info!("{} service stopped gracefully", self.service.service_name()),
                Ok(Ok(Err(e))) => warn!("{} service stopped with error: {}", self.service.service_name(), e),
                Ok(Err(panic_error)) => error!("{} service panicked: {}", self.service.service_name(), panic_error),
                Err(_) => return Err(ServiceError::ShutdownTimeout),
            }
        }

        *self.status.write() = ServiceStatus::Stopped;
        self.start_time = None;
        Ok(())
    }

    /// Restart the service
    pub async fn restart(&mut self) -> Result<(), ServiceError> {
        self.stop().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.start().await
    }

    pub fn is_running(&self) -> bool {
        matches!(*self.status.read(), ServiceStatus::Running | ServiceStatus::Starting)
    }

    pub fn status(&self) -> ServiceStatus {
        self.status.read().clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("Service '{0}' is already running")]
    AlreadyRunning(String),
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    #[error("Service initialization failed: {0}")]
    InitializationFailed(String),
    #[error("Service shutdown timeout")]
    ShutdownTimeout,
}
```

## Logging Configuration

### Integration with Existing Logging System

The background service abstraction integrates with voice-cli's existing configurable logging system. The log directory is configured via the `log_dir` setting in configuration files, not hardcoded.

```rust
use crate::utils::init_logging;
use crate::models::Config;

/// Initialize logging for background services using existing voice-cli infrastructure
pub fn init_service_logging(config: &Config, service_name: &str, foreground_mode: bool) -> Result<(), Box<dyn std::error::Error>> {
    if foreground_mode {
        // Foreground mode: Use existing init_logging which logs to both console and file
        // The existing implementation already handles:
        // - Console output with proper formatting
        // - File output to config.log_dir_path() 
        // - Daily log rotation
        // - Configurable log levels
        init_logging(config)?;
        
        // Inform user where logs are also being written
        let log_dir = config.log_dir_path();
        println!("📋 Logs are also written to: {}", log_dir.join("voice-cli.log").display());
    } else {
        // Background mode: Use existing logging but suppress console output
        // The existing init_logging already handles file-only logging properly
        init_logging(config)?;
    }
    
    Ok(())
}

/// Get the configured logs directory (uses existing method)
pub fn get_logs_directory(config: &Config) -> std::path::PathBuf {
    config.log_dir_path()  // Uses the existing log_dir_path() method
}
```

### Configuration Templates

The existing configuration templates already properly configure logging:

**Server Config (`templates/server-config.yml.template`):**
```yaml
logging:
  level: "info"
  log_dir: "./logs"           # Configurable, not hardcoded
  max_file_size: "10MB"
  max_files: 5
```

**Cluster Config (`templates/cluster-config.yml.template`):**
```yaml
logging:
  level: "info"
  log_dir: "./logs"           # Configurable, not hardcoded  
  max_file_size: "10MB"
  max_files: 5
```

**Load Balancer Config (`templates/lb-config.yml.template`):**
```yaml
logging:
  level: "info"
  log_dir: "./logs"           # Configurable, not hardcoded
  max_file_size: "10MB"
  max_files: 5
```

### Environment Override Support

The existing system already supports environment variable overrides:

```bash
# Override log directory via environment variable
export VOICE_CLI_LOG_DIR="/var/log/voice-cli"
voice-cli server start  # Will use /var/log/voice-cli instead of ./logs

# Override log level
export VOICE_CLI_LOG_LEVEL="debug"
voice-cli cluster start  # Will use debug level logging
```

## Service Implementations

### HTTP Server Service

```rust
#[derive(Clone)]
pub struct HttpServerService {
    config: Option<crate::models::Config>,
    foreground_mode: bool,  // true for 'run', false for 'start'
}

impl BackgroundService for HttpServerService {
    type Config = crate::models::Config;
    type Error = crate::VoiceCliError;

    fn service_name(&self) -> &'static str {
        "http-server"
    }

    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error> {
        // Initialize logging using existing voice-cli infrastructure
        init_service_logging(&config, "http-server", self.foreground_mode)
            .map_err(|e| crate::VoiceCliError::Config(format!("Logging init failed: {}", e)))?;
            
        info!("Initializing HTTP server in {} mode", 
              if self.foreground_mode { "foreground" } else { "background" });
        info!("Log directory: {}", config.log_dir_path().display());
              
        self.config = Some(config);
        Ok(())
    }

    async fn run(&mut self, mut shutdown_rx: broadcast::Receiver<()>) -> Result<(), Self::Error> {
        let config = self.config.as_ref().unwrap().clone();
        
        // Create server future
        let server_future = crate::server::create_cluster_aware_server_with_shutdown(config).await?;

        // Combined shutdown signal handling
        let shutdown_signal = async {
            tokio::select! {
                _ = shutdown_rx.recv() => info!("Received shutdown signal via channel"),
                _ = Self::system_signals() => info!("Received system shutdown signal"),
            }
        };

        // Run server with shutdown monitoring
        tokio::select! {
            result = server_future => {
                result.map_err(|e| crate::VoiceCliError::Config(format!("Server error: {}", e)))
            }
            _ = shutdown_signal => {
                info!("HTTP server shutting down gracefully");
                Ok(())
            }
        }
    }

    async fn health_check(&self) -> ServiceHealth {
        // Implement health check via HTTP request
        ServiceHealth::Healthy
    }

    fn validate_config(config: &Self::Config) -> Result<(), Self::Error> {
        if config.server.host.is_empty() {
            return Err(crate::VoiceCliError::Config("Server host cannot be empty".to_string()));
        }
        Ok(())
    }
}

impl HttpServerService {
    pub fn new(foreground_mode: bool) -> Self {
        Self { 
            config: None,
            foreground_mode,
        }
    }

    async fn system_signals() {
        let ctrl_c = async {
            tokio::signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
            term.recv().await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => info!("Received Ctrl+C"),
            _ = terminate => info!("Received SIGTERM"),
        }
    }
}
```

### Cluster Node Service

```rust
#[derive(Clone)]
pub struct ClusterNodeService {
    config: Option<crate::models::Config>,
    node_id: String,
    foreground_mode: bool,  // true for 'run', false for 'start'
}

impl BackgroundService for ClusterNodeService {
    type Config = crate::models::Config;
    type Error = crate::VoiceCliError;

    fn service_name(&self) -> &'static str {
        "cluster-node"
    }

    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error> {
        // Initialize logging using existing voice-cli infrastructure
        init_service_logging(&config, "cluster-node", self.foreground_mode)
            .map_err(|e| crate::VoiceCliError::Config(format!("Logging init failed: {}", e)))?;
            
        info!("Initializing cluster node '{}' in {} mode", 
              self.node_id,
              if self.foreground_mode { "foreground" } else { "background" });
        info!("Log directory: {}", config.log_dir_path().display());
              
        self.config = Some(config);
        Ok(())
    }

    async fn run(&mut self, mut shutdown_rx: broadcast::Receiver<()>) -> Result<(), Self::Error> {
        let config = self.config.as_ref().unwrap().clone();
        
        // Create cluster service manager
        let cluster_state = Arc::new(crate::cluster::ClusterState::new());
        let service_manager = crate::cluster::ClusterServiceManager::new(config, cluster_state);

        // Create cancellation token
        let cancellation_token = tokio_util::sync::CancellationToken::new();
        let token_clone = cancellation_token.clone();

        // Start cluster services
        let cluster_future = async move {
            service_manager.start().await
                .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))
        };

        // Monitor shutdown signals
        let shutdown_monitor = async move {
            tokio::select! {
                _ = shutdown_rx.recv() => info!("Cluster received shutdown signal via channel"),
                _ = Self::system_signals() => info!("Cluster received system shutdown signal"),
            }
            token_clone.cancel();
        };

        tokio::select! {
            result = cluster_future => result,
            _ = shutdown_monitor => {
                info!("Cluster node shutting down gracefully");
                Ok(())
            }
        }
    }

    async fn health_check(&self) -> ServiceHealth {
        ServiceHealth::Healthy
    }

    fn validate_config(config: &Self::Config) -> Result<(), Self::Error> {
        if !config.cluster.enabled {
            return Err(crate::VoiceCliError::Config("Cluster mode must be enabled".to_string()));
        }
        Ok(())
    }
}

impl ClusterNodeService {
    pub fn new(node_id: String, foreground_mode: bool) -> Self {
        Self { 
            config: None, 
            node_id,
            foreground_mode,
        }
    }

    async fn system_signals() {
        // Same implementation as HttpServerService
        let ctrl_c = async {
            tokio::signal::ctrl_c().await.expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term = signal(SignalKind::terminate()).expect("Failed to install SIGTERM handler");
            term.recv().await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => info!("Received Ctrl+C"),
            _ = terminate => info!("Received SIGTERM"),
        }
    }
}
```

## CLI Integration

### Updated Command Handlers

```rust
/// Updated CLI handlers using the unified service manager
pub mod handlers {
    use super::*;

    /// Run server in foreground mode (for testing)
    pub async fn handle_server_run(config: &crate::models::Config) -> crate::Result<()> {
        let service = HttpServerService::new(true);  // foreground mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        info!("Starting HTTP server in foreground mode");
        info!("Press Ctrl+C to stop the server");
        
        // In foreground mode, we run directly without detaching
        manager.start().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        // Wait for the service to complete
        while manager.is_running() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        info!("HTTP server stopped");
        Ok(())
    }

    /// Start server as background daemon
    pub async fn handle_server_start(config: &crate::models::Config) -> crate::Result<()> {
        let service = HttpServerService::new(false);  // background mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.start().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Server daemon started successfully");
        let log_dir = get_logs_directory(&config);
        info!("Logs are written to: {}", log_dir.join("voice-cli.log").display());
        Ok(())
    }

    pub async fn handle_server_stop(config: &crate::models::Config) -> crate::Result<()> {
        let service = HttpServerService::new(false);  // background mode for stop
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.stop().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Server daemon stopped successfully");
        Ok(())
    }

    pub async fn handle_server_restart(config: &crate::models::Config) -> crate::Result<()> {
        let service = HttpServerService::new(false);  // background mode for restart
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.restart().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Server daemon restarted successfully");
        let log_dir = get_logs_directory(&config);
        info!("Logs are written to: {}", log_dir.join("voice-cli.log").display());
        Ok(())
    }

    /// Run cluster node in foreground mode (for testing)
    pub async fn handle_cluster_run(
        config: &crate::models::Config,
        node_id: String,
    ) -> crate::Result<()> {
        let service = ClusterNodeService::new(node_id.clone(), true);  // foreground mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        info!("Starting cluster node '{}' in foreground mode", node_id);
        info!("Press Ctrl+C to stop the cluster node");
        
        manager.start().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        // Wait for the service to complete
        while manager.is_running() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        info!("Cluster node '{}' stopped", node_id);
        Ok(())
    }

    pub async fn handle_cluster_start(
        config: &crate::models::Config,
        node_id: String,
    ) -> crate::Result<()> {
        let service = ClusterNodeService::new(node_id.clone(), false);  // background mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.start().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Cluster node '{}' daemon started successfully", node_id);
        let log_dir = get_logs_directory(&config);
        info!("Logs are written to: {}", log_dir.join("voice-cli.log").display());
        Ok(())
    }

    pub async fn handle_cluster_stop(config: &crate::models::Config) -> crate::Result<()> {
        let service = ClusterNodeService::new("default".to_string(), false);  // background mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.stop().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Cluster node daemon stopped successfully");
        Ok(())
    }

    pub async fn handle_cluster_restart(
        config: &crate::models::Config,
        node_id: String,
    ) -> crate::Result<()> {
        let service = ClusterNodeService::new(node_id.clone(), false);  // background mode
        let mut manager = ServiceManager::new(service, config.clone());
        
        manager.restart().await
            .map_err(|e| crate::VoiceCliError::Daemon(e.to_string()))?;
            
        info!("Cluster node '{}' daemon restarted successfully", node_id);
        let log_dir = get_logs_directory(&config);
        info!("Logs are written to: {}", log_dir.join("voice-cli.log").display());
        Ok(())
    }
}
```

## Migration Strategy

### Phase 1: Core Implementation
1. Implement `BackgroundService` trait and `ServiceManager`
2. Create `HttpServerService` and integrate with existing server logic
3. Update `handle_server_start/stop/restart` to use new abstraction

### Phase 2: Cluster Integration
1. Implement `ClusterNodeService`
2. Update cluster command handlers
3. Maintain full backward compatibility

### Phase 3: Cleanup
1. Remove old `DaemonService` implementations
2. Add comprehensive testing
3. Documentation updates

## Key Benefits

1. **Unified Interface**: All background services follow the same patterns
2. **Memory Safety**: No unsafe code, proper resource management
3. **Signal Handling**: Consistent Ctrl+C and SIGTERM handling
4. **Graceful Shutdown**: 30-second timeout with proper cleanup
5. **Error Handling**: Standardized error types and logging
6. **Testing**: Easy to mock and test individual services
7. **Maintainability**: Single source of truth for daemon lifecycle logic

This abstraction eliminates the problems with process spawning and provides a robust, safe foundation for all background services in voice-cli.

## Usage Examples

### Foreground Mode (Testing)

```bash
# Run HTTP server in foreground for testing
$ voice-cli server run
📋 Logs are also written to: ./logs/voice-cli.log
[INFO] Starting HTTP server in foreground mode
[INFO] Log directory: ./logs
[INFO] HTTP server listening on 0.0.0.0:8080
[INFO] Press Ctrl+C to stop the server
# ... server logs appear in real-time ...
^C
[INFO] Received Ctrl+C
[INFO] HTTP server shutting down gracefully
[INFO] HTTP server stopped

# Run cluster node in foreground for testing  
$ voice-cli cluster run --node-id test-node
📋 Logs are also written to: ./logs/voice-cli.log
[INFO] Starting cluster node 'test-node' in foreground mode
[INFO] Log directory: ./logs
[INFO] gRPC server listening on 0.0.0.0:50051
[INFO] Press Ctrl+C to stop the cluster node
# ... cluster logs appear in real-time ...
^C
[INFO] Received Ctrl+C
[INFO] Cluster node 'test-node' shutting down gracefully
[INFO] Cluster node 'test-node' stopped
```

### Background Mode (Production)

```bash
# Start HTTP server as background daemon
$ voice-cli server start
[INFO] Server daemon started successfully
[INFO] Logs are written to: ./logs/voice-cli.log

# Check what's in the configured logs directory
$ ls -la ./logs/  # Uses configured log_dir from config file
voice-cli.log
voice-cli.log.2024-01-14  # Daily rotated logs

# Start cluster node as background daemon
$ voice-cli cluster start --node-id prod-node
[INFO] Cluster node 'prod-node' daemon started successfully
[INFO] Logs are written to: ./logs/voice-cli.log

# View logs from configured directory
$ tail -f ./logs/voice-cli.log
2024-01-15T10:30:15.123456Z [INFO] HTTP server listening on 0.0.0.0:8080
2024-01-15T10:30:15.234567Z [INFO] Health check endpoint available at /health
2024-01-15T10:31:20.345678Z [INFO] Received transcription request

# Use different log directory via environment variable
$ export VOICE_CLI_LOG_DIR="/var/log/voice-cli"
$ voice-cli server start
[INFO] Server daemon started successfully
[INFO] Logs are written to: /var/log/voice-cli/voice-cli.log

# Stop services
$ voice-cli server stop
[INFO] Server daemon stopped successfully

$ voice-cli cluster stop
[INFO] Cluster node daemon stopped successfully
```

### Log Directory Structure

```
# Default configuration (log_dir: "./logs")
./
├── voice-cli                    # Binary
├── config.yml                  # Configuration file
└── logs/                        # Configurable logs directory
    ├── voice-cli.log            # All services log to same file
    └── voice-cli.log.2024-01-14 # Daily rotated logs

# Custom configuration (log_dir: "/var/log/voice-cli")
/var/log/voice-cli/
├── voice-cli.log            # Centralized logging
└── voice-cli.log.2024-01-14 # Rotated logs

# Environment override (VOICE_CLI_LOG_DIR="/tmp/logs")
/tmp/logs/
├── voice-cli.log            # All services use same log file
└── voice-cli.log.2024-01-14 # Daily rotation
```

## Cargo.toml Configuration

```toml
[package]
name = "voice-cli"
version = "0.1.0"
edition = "2024"  # Required for native async traits
rust-version = "1.85"  # Minimum Rust version

[dependencies]
tokio = { version = "1.0", features = ["full"] }
parking_lot = "0.12"
thiserror = "1.0"
tracing = "0.1"
# Note: tracing-subscriber and tracing-appender already included in existing voice-cli dependencies
# async-trait = "0.1"  # No longer needed with Rust 1.85 + edition 2024

# Existing voice-cli dependencies
axum = "0.8"
serde = { version = "1.0", features = ["derive"] }
# ... other existing dependencies
```