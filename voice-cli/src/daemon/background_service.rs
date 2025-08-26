//! Unified background service abstraction for voice-cli
//! 
//! This module provides a modern, safe, and unified approach to managing background services
//! across different voice-cli commands (server, cluster, load balancer). It follows Rust
//! best practices and avoids unsafe code completely.
//!
//! # Features
//! - In-process architecture using Tokio
//! - Zero unsafe code
//! - Graceful lifecycle management
//! - Unified interface across service types
//! - Support for both foreground (run) and background (start/restart) modes
//! - Centralized logging with configurable directories
//! - Proper signal handling for graceful shutdown

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use parking_lot::RwLock;
use tracing::{info, warn, error, debug};

/// Unified background service trait for all voice-cli services
/// 
/// This trait uses Rust 1.85+ native async trait support (edition = "2024")
/// No need for #[async_trait] annotation anymore
pub trait BackgroundService: Send + Sync + 'static {
    type Config: Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    /// Service identifier for logging and management
    fn service_name(&self) -> &'static str;

    /// Initialize service with configuration
    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error>;

    /// Run the main service loop (cancellation-aware)
    /// This method should listen for the shutdown signal and exit gracefully
    /// The future must be Send to work with tokio::spawn
    fn run(&mut self, shutdown_rx: broadcast::Receiver<()>) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Perform cleanup before shutdown (optional)
    /// The future must be Send to work with tokio::spawn
    fn cleanup(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }

    /// Health check implementation
    async fn health_check(&self) -> ServiceHealth;

    /// Configuration validation (called before initialization)
    fn validate_config(config: &Self::Config) -> Result<(), Self::Error>;
}

/// Service health status
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceHealth {
    Healthy,
    Unhealthy { reason: String },
    Unknown,
}

/// Service runtime status
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceStatus {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed { error: String },
}

/// Universal service manager for all background services
/// 
/// This manager handles the lifecycle of any service implementing BackgroundService
pub struct ServiceManager<S: BackgroundService + Clone> {
    service: S,
    config: S::Config,
    status: Arc<RwLock<ServiceStatus>>,
    shutdown_tx: Option<broadcast::Sender<()>>,
    service_handle: Option<JoinHandle<Result<(), ServiceError>>>,
    start_time: Option<Instant>,
    foreground_mode: bool,
}

impl<S: BackgroundService + Clone> ServiceManager<S> {
    /// Create a new service manager
    /// 
    /// # Arguments
    /// * `service` - The service implementation
    /// * `config` - Service configuration
    /// * `foreground_mode` - true for 'run' command, false for 'start'/'restart'
    pub fn new(service: S, config: S::Config, foreground_mode: bool) -> Self {
        Self {
            service,
            config,
            status: Arc::new(RwLock::new(ServiceStatus::Stopped)),
            shutdown_tx: None,
            service_handle: None,
            start_time: None,
            foreground_mode,
        }
    }

    /// Start the service
    pub async fn start(&mut self) -> Result<(), ServiceError> {
        info!("ServiceManager.start() called");
        if self.is_running() {
            info!("Service is already running, returning error");
            return Err(ServiceError::AlreadyRunning(self.service.service_name().to_string()));
        }

        let service_name = self.service.service_name();
        let mode = if self.foreground_mode { "foreground" } else { "background" };
        
        info!("Starting {} service in {} mode", service_name, mode);
        *self.status.write() = ServiceStatus::Starting;

        // Validate configuration
        info!("Validating configuration for {}", service_name);
        S::validate_config(&self.config)
            .map_err(|e| {
                error!("Configuration validation failed: {}", e);
                ServiceError::ConfigurationError(e.to_string())
            })?;
        info!("Configuration validation passed for {}", service_name);

        // Initialize service
        info!("Initializing service: {}", service_name);
        self.service.initialize(self.config.clone()).await
            .map_err(|e| {
                error!("Service initialization failed: {}", e);
                ServiceError::InitializationFailed(e.to_string())
            })?;
        info!("Service initialization completed for {}", service_name);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        // Start the service task
        info!("About to start service task for {}", service_name);
        self.start_service_task(shutdown_rx).await?;
        info!("Service task started successfully for {}", service_name);

        self.start_time = Some(Instant::now());
        info!("{} service started successfully in {} mode", service_name, mode);
        
        Ok(())
    }

    /// Internal method to start the service task
    async fn start_service_task(&mut self, shutdown_rx: broadcast::Receiver<()>) -> Result<(), ServiceError> {
        let service_name = self.service.service_name();
        let status_clone = self.status.clone();
        
        // Clone service for background task (requires Clone trait)
        let mut service_clone = self.service.clone();

        // Spawn service task
        let service_handle = tokio::spawn(async move {
            info!("Service task started for {}", service_name);
            *status_clone.write() = ServiceStatus::Running;
            
            info!("About to run service: {}", service_name);
            // Run the actual service
            let result: Result<(), S::Error> = service_clone.run(shutdown_rx).await;
            
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
            
            result.map_err(|e| ServiceError::ServiceError(e.to_string()))
        });

        self.service_handle = Some(service_handle);
        Ok(())
    }

    /// Stop the service gracefully
    pub async fn stop(&mut self) -> Result<(), ServiceError> {
        if !self.is_running() {
            debug!("{} service is not running", self.service.service_name());
            return Ok(());
        }

        let service_name = self.service.service_name();
        info!("Stopping {} service", service_name);
        *self.status.write() = ServiceStatus::Stopping;

        // Send shutdown signal
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Wait for service to stop with timeout
        if let Some(service_handle) = self.service_handle.take() {
            match tokio::time::timeout(Duration::from_secs(30), service_handle).await {
                Ok(Ok(_)) => {
                    info!("{} service stopped gracefully", service_name);
                }
                Ok(Err(e)) => {
                    warn!("{} service stopped with error: {}", service_name, e);
                }
                Err(_) => {
                    error!("{} service shutdown timeout", service_name);
                    return Err(ServiceError::ShutdownTimeout);
                }
            }
        }

        // Perform cleanup
        if let Err(e) = self.service.cleanup().await {
            warn!("Cleanup error for {}: {}", service_name, e);
        }

        *self.status.write() = ServiceStatus::Stopped;
        self.start_time = None;
        info!("{} service stopped successfully", service_name);
        
        Ok(())
    }

    /// Restart the service
    pub async fn restart(&mut self) -> Result<(), ServiceError> {
        info!("Restarting {} service", self.service.service_name());
        
        // Stop if running
        if self.is_running() {
            self.stop().await?;
        }

        // Small delay to ensure cleanup
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Start again
        self.start().await
    }

    /// Check if service is running
    pub fn is_running(&self) -> bool {
        matches!(*self.status.read(), ServiceStatus::Running | ServiceStatus::Starting)
    }

    /// Get current service status
    pub fn status(&self) -> ServiceStatus {
        self.status.read().clone()
    }

    /// Get service uptime
    pub fn uptime(&self) -> Option<Duration> {
        self.start_time.map(|start| start.elapsed())
    }

    /// Get service health
    pub async fn health(&self) -> ServiceHealth {
        if self.is_running() {
            self.service.health_check().await
        } else {
            ServiceHealth::Unhealthy { 
                reason: "Service is not running".to_string() 
            }
        }
    }

    /// Get service name
    pub fn service_name(&self) -> &'static str {
        self.service.service_name()
    }

    /// Check if running in foreground mode
    pub fn is_foreground_mode(&self) -> bool {
        self.foreground_mode
    }

    /// Wait for service to complete (blocks until service stops)
    /// This is used in background mode to keep the process alive
    pub async fn wait(&mut self) {
        if let Some(handle) = self.service_handle.take() {
            let _ = handle.await;
        }
    }
}

/// Service management errors
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
    
    #[error("Service error: {0}")]
    ServiceError(String),
    
    #[error("Daemonization error: {0}")]
    DaemonError(String),
}

/// Trait for services that can be cloned for background execution
/// This is a helper trait to work around the BackgroundService cloning issue
pub trait ClonableService: BackgroundService + Clone {}

/// A wrapper that makes it easier to work with cloneable services
pub struct ClonableServiceManager<S: ClonableService> {
    inner: ServiceManager<S>,
}

impl<S: ClonableService> ClonableServiceManager<S> {
    pub fn new(service: S, config: S::Config, foreground_mode: bool) -> Self {
        Self {
            inner: ServiceManager::new(service, config, foreground_mode),
        }
    }

    /// Start the service (properly handles cloning)
    pub async fn start(&mut self) -> Result<(), ServiceError> {
        if self.inner.is_running() {
            return Err(ServiceError::AlreadyRunning(self.inner.service.service_name().to_string()));
        }

        let service_name = self.inner.service.service_name();
        let mode = if self.inner.foreground_mode { "foreground" } else { "background" };
        
        info!("Starting {} service in {} mode", service_name, mode);
        *self.inner.status.write() = ServiceStatus::Starting;

        // Validate configuration
        S::validate_config(&self.inner.config)
            .map_err(|e| ServiceError::ConfigurationError(e.to_string()))?;

        // Initialize service
        self.inner.service.initialize(self.inner.config.clone()).await
            .map_err(|e| ServiceError::InitializationFailed(e.to_string()))?;

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        self.inner.shutdown_tx = Some(shutdown_tx);

        // Clone service for background task
        let mut service_clone = self.inner.service.clone();
        let status_clone = self.inner.status.clone();

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
                    Ok(())
                }
                Err(e) => {
                    error!("{} service failed: {}", service_name, e);
                    *status_clone.write() = ServiceStatus::Failed { 
                        error: e.to_string() 
                    };
                    Err(ServiceError::ServiceError(e.to_string()))
                }
            }
        });

        self.inner.service_handle = Some(service_handle);
        self.inner.start_time = Some(Instant::now());

        info!("{} service started successfully in {} mode", service_name, mode);
        Ok(())
    }

    /// Delegate other methods to inner manager
    pub async fn stop(&mut self) -> Result<(), ServiceError> {
        self.inner.stop().await
    }

    pub async fn restart(&mut self) -> Result<(), ServiceError> {
        self.inner.restart().await
    }

    pub fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    pub fn status(&self) -> ServiceStatus {
        self.inner.status()
    }

    pub fn uptime(&self) -> Option<Duration> {
        self.inner.uptime()
    }

    pub async fn health(&self) -> ServiceHealth {
        self.inner.health().await
    }

    pub fn service_name(&self) -> &'static str {
        self.inner.service_name()
    }

    pub fn is_foreground_mode(&self) -> bool {
        self.inner.is_foreground_mode()
    }

    /// Wait for service to complete (blocks until service stops)
    /// This is used in background mode to keep the process alive
    pub async fn wait(&mut self) {
        self.inner.wait().await
    }
}

impl<S: BackgroundService + Clone> Drop for ServiceManager<S> {
    fn drop(&mut self) {
        // Only auto-shutdown in foreground mode
        // In background mode, the service should continue running until explicitly stopped
        if self.foreground_mode && self.is_running() {
            // Send shutdown signal for foreground services
            if let Some(shutdown_tx) = self.shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
        }
    }
}

/// Convenience type alias for the most common service manager
pub type DefaultServiceManager<S> = ClonableServiceManager<S>;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    // Mock service for testing
    #[derive(Clone)]
    struct MockService {
        name: &'static str,
        should_fail: bool,
    }

    impl MockService {
        fn new(name: &'static str) -> Self {
            Self { name, should_fail: false }
        }

        fn with_failure(name: &'static str) -> Self {
            Self { name, should_fail: true }
        }
    }

    #[derive(Clone)]
    struct MockConfig {
        valid: bool,
    }

    impl Default for MockConfig {
        fn default() -> Self {
            Self { valid: true }
        }
    }

    // Create a simple error type for MockService
    #[derive(Debug, Clone)]
    struct MockServiceError(String);
    
    impl std::fmt::Display for MockServiceError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "MockService error: {}", self.0)
        }
    }
    
    impl std::error::Error for MockServiceError {}
    
    impl From<&str> for MockServiceError {
        fn from(s: &str) -> Self {
            MockServiceError(s.to_string())
        }
    }

    impl BackgroundService for MockService {
        type Config = MockConfig;
        type Error = MockServiceError;

        fn service_name(&self) -> &'static str {
            self.name
        }

        async fn initialize(&mut self, _config: Self::Config) -> Result<(), Self::Error> {
            sleep(Duration::from_millis(10)).await;
            Ok(())
        }

        fn run(&mut self, mut shutdown_rx: broadcast::Receiver<()>) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
            let should_fail = self.should_fail;
            async move {
                if should_fail {
                    return Err(MockServiceError::from("Mock service failure"));
                }

                // Simulate service work
                tokio::select! {
                    _ = sleep(Duration::from_secs(10)) => {
                        // Service completed normally (shouldn't happen in tests)
                        Ok(())
                    }
                    _ = shutdown_rx.recv() => {
                        // Received shutdown signal
                        Ok(())
                    }
                }
            }
        }

        async fn health_check(&self) -> ServiceHealth {
            if self.should_fail {
                ServiceHealth::Unhealthy {
                    reason: "Mock service is configured to fail".to_string(),
                }
            } else {
                ServiceHealth::Healthy
            }
        }

        fn validate_config(config: &Self::Config) -> Result<(), Self::Error> {
            if config.valid {
                Ok(())
            } else {
                Err(MockServiceError::from("Invalid mock configuration"))
            }
        }
    }

    impl ClonableService for MockService {}

    #[tokio::test]
    async fn test_service_lifecycle() {
        let service = MockService::new("test-service");
        let config = MockConfig::default();
        let mut manager = DefaultServiceManager::new(service, config, false);

        // Initially stopped
        assert!(!manager.is_running());
        assert!(matches!(manager.status(), ServiceStatus::Stopped));

        // Start service
        manager.start().await.unwrap();
        assert!(manager.is_running());

        // Stop service
        manager.stop().await.unwrap();
        assert!(!manager.is_running());
        assert!(matches!(manager.status(), ServiceStatus::Stopped));
    }

    #[tokio::test]
    async fn test_service_restart() {
        let service = MockService::new("restart-test");
        let config = MockConfig::default();
        let mut manager = DefaultServiceManager::new(service, config, false);

        // Start and restart
        manager.start().await.unwrap();
        assert!(manager.is_running());

        manager.restart().await.unwrap();
        assert!(manager.is_running());

        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn test_invalid_config() {
        let service = MockService::new("config-test");
        let config = MockConfig { valid: false };
        let mut manager = DefaultServiceManager::new(service, config, false);

        let result = manager.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_double_start() {
        let service = MockService::new("double-start");
        let config = MockConfig::default();
        let mut manager = DefaultServiceManager::new(service, config, false);

        manager.start().await.unwrap();
        
        let result = manager.start().await;
        assert!(matches!(result, Err(ServiceError::AlreadyRunning(_))));

        manager.stop().await.unwrap();
    }
}