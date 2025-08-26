# Voice-CLI Unified Background Service Abstraction

## Overview

The Voice-CLI project has been enhanced with a modern, unified background service abstraction that provides consistent, safe, and efficient management of background services across different commands (`voice-cli server start`, `voice-cli cluster start`, `voice-cli lb start`, etc.).

This new system replaces the previous daemon implementations with a modern Rust approach that follows current best practices and avoids unsafe code completely.

## Key Features

### ✅ Modern Rust Design
- **Rust 1.85+ Native Async Traits**: Uses `edition = "2024"`, no more `#[async_trait]` needed
- **Zero Unsafe Code**: Complete memory safety without any `unsafe` blocks
- **In-Process Architecture**: All services run within the same process using Tokio
- **Ownership-Based Resource Management**: Leverages Rust's ownership system for cleanup

### ✅ Unified Interface
- **Consistent API**: Same interface for HTTP server, cluster nodes, and load balancers
- **Command Mode Support**: Both `run` (foreground) and `start`/`restart` (background) modes
- **Standardized Lifecycle**: Initialize → Run → Cleanup pattern for all services
- **Health Monitoring**: Built-in health checks and status reporting

### ✅ Robust Operations
- **Graceful Shutdown**: Proper signal handling and resource cleanup
- **Error Recovery**: Comprehensive error handling and recovery mechanisms
- **Logging Integration**: Works with existing voice-cli logging infrastructure
- **Configuration Validation**: Pre-startup validation of all configurations

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    CLI Commands                              │
├─────────────────────────────────────────────────────────────┤
│  voice-cli server start  │  voice-cli cluster start         │
│  voice-cli server run    │  voice-cli lb start              │
└─────────────────┬───────────────────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────────────────┐
│               Unified Handlers                               │
│  (cli/unified_handlers.rs)                                  │
└─────────────────┬───────────────────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────────────────┐
│              Service Manager                                 │
│  DefaultServiceManager<T: BackgroundService>                │
└─────────────────┬───────────────────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────────────────┐
│           Background Services                                │
├─────────────────┬─────────────────┬─────────────────────────┤
│ HttpServerService│ ClusterNodeService│ LoadBalancerService   │
│                 │                 │                         │
│ - HTTP server   │ - gRPC cluster  │ - Traffic distribution │
│ - Health checks │ - Node management│ - Health monitoring    │
│ - Graceful stop │ - Leader election│ - Circuit breaker     │
└─────────────────┴─────────────────┴─────────────────────────┘
```

## Core Components

### BackgroundService Trait

The foundation of the new system is the `BackgroundService` trait:

```rust
pub trait BackgroundService: Send + Sync + 'static {
    type Config: Clone + Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + 'static;

    fn service_name(&self) -> &'static str;
    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error>;
    async fn run(&mut self, shutdown_rx: broadcast::Receiver<()>) -> Result<(), Self::Error>;
    async fn cleanup(&mut self) -> Result<(), Self::Error>;
    async fn health_check(&self) -> ServiceHealth;
    fn validate_config(config: &Self::Config) -> Result<(), Self::Error>;
}
```

### Service Manager

The `DefaultServiceManager` provides lifecycle management for any service implementing `BackgroundService`:

```rust
pub struct DefaultServiceManager<S: ClonableService> {
    // ... internal state
}

impl<S: ClonableService> DefaultServiceManager<S> {
    pub async fn start(&mut self) -> Result<(), ServiceError>;
    pub async fn stop(&mut self) -> Result<(), ServiceError>;
    pub async fn restart(&mut self) -> Result<(), ServiceError>;
    pub fn is_running(&self) -> bool;
    pub fn status(&self) -> ServiceStatus;
    pub async fn health(&self) -> ServiceHealth;
    pub fn uptime(&self) -> Option<Duration>;
}
```

### Service Implementations

Three concrete service implementations are provided:

1. **HttpServerService**: Manages HTTP server with optional cluster awareness
2. **ClusterNodeService**: Manages cluster nodes with gRPC communication
3. **LoadBalancerService**: Manages load balancing across cluster nodes

## Quick Start Guide

### 1. Basic HTTP Server

```rust
use voice_cli::daemon::{HttpServerServiceBuilder, DefaultServiceManager};

// Create service
let service = HttpServerServiceBuilder::new()
    .foreground_mode(false) // Background mode
    .build();

// Create manager
let mut manager = DefaultServiceManager::new(service, config, false);

// Lifecycle operations
manager.start().await?;
println!("Status: {:?}", manager.status());
println!("Health: {:?}", manager.health().await);
manager.stop().await?;
```

### 2. Cluster Node

```rust
use voice_cli::daemon::{ClusterNodeServiceBuilder, DefaultServiceManager};

// Create cluster node with auto-generated ID
let service = ClusterNodeServiceBuilder::new()
    .auto_node_id()
    .foreground_mode(false)
    .build()?;

let mut manager = DefaultServiceManager::new(service, config, false);
manager.start().await?;
```

### 3. Load Balancer

```rust
use voice_cli::daemon::{LoadBalancerServiceBuilder, DefaultServiceManager};

let service = LoadBalancerServiceBuilder::new()
    .foreground_mode(false)
    .build();

let mut manager = DefaultServiceManager::new(service, config, false);
manager.start().await?;
```

## Command Integration

### New Unified Handlers

The system provides new CLI handlers in `cli/unified_handlers.rs`:

```rust
// HTTP Server commands
voice_cli::cli::unified_server::handle_run(config).await?;    // foreground
voice_cli::cli::unified_server::handle_start(config).await?;  // background
voice_cli::cli::unified_server::handle_stop(config).await?;
voice_cli::cli::unified_server::handle_restart(config).await?;
voice_cli::cli::unified_server::handle_status(config).await?;

// Cluster commands
voice_cli::cli::unified_cluster::handle_run(config, node_id).await?;
voice_cli::cli::unified_cluster::handle_start(config, node_id).await?;
// ... etc

// Load Balancer commands
voice_cli::cli::unified_lb::handle_run(config).await?;
voice_cli::cli::unified_lb::handle_start(config).await?;
// ... etc
```

### Integration Example

To integrate the new handlers into your CLI routing:

```rust
// In main.rs or CLI handler
match action {
    ServerAction::Run { .. } => {
        // Use new unified handler
        voice_cli::cli::unified_server::handle_run(config).await
    }
    ServerAction::Start { .. } => {
        voice_cli::cli::unified_server::handle_start(config).await
    }
    // ... other actions
}
```

## Logging Integration

The new system integrates seamlessly with voice-cli's existing logging infrastructure:

### Configurable Log Directories

Logs are written to the directory specified in your configuration:

```yaml
logging:
  level: "info"
  log_dir: "./logs"  # Configurable, not hardcoded
  max_file_size: "10MB"
  max_files: 5
```

### Environment Variable Override

```bash
# Override log directory
export VOICE_CLI_LOG_DIR="/var/log/voice-cli"
voice-cli server start

# Override log level
export VOICE_CLI_LOG_LEVEL="debug"
voice-cli cluster start
```

### Foreground vs Background Logging

- **Foreground mode** (`run` commands): Logs to both console and file
- **Background mode** (`start`/`restart` commands): Logs to file only

## Migration Guide

### From Legacy Daemon to New Service

#### Old Approach (Deprecated)

```rust
// ❌ Old way - deprecated
use voice_cli::daemon::DaemonService;

let daemon = DaemonService::new(config);
daemon.start_daemon().await?;
daemon.stop_daemon().await?;
```

#### New Approach (Recommended)

```rust
// ✅ New way - recommended
use voice_cli::daemon::{HttpServerServiceBuilder, DefaultServiceManager};

let service = HttpServerServiceBuilder::new()
    .foreground_mode(false)
    .build();
let mut manager = DefaultServiceManager::new(service, config, false);

manager.start().await?;
manager.stop().await?;
```

#### Migration Helper

For easy migration, use the provided helper:

```rust
use voice_cli::daemon::migration;

// Quick migration from legacy daemon
let mut manager = migration::create_http_server_manager(config, false);
manager.start().await?;
```

### CLI Handler Migration

#### Update Command Handlers

Replace existing daemon-based handlers with unified handlers:

```rust
// Before
pub async fn handle_server_start(config: &Config) -> Result<()> {
    let daemon_service = DaemonService::new(config.clone());
    daemon_service.start_daemon().await
}

// After
pub async fn handle_server_start(config: &Config) -> Result<()> {
    voice_cli::cli::unified_server::handle_start(config).await
}
```

### Configuration Changes

No configuration file changes are required. The new system uses the same configuration structure as before.

## Advanced Usage

### Custom Service Implementation

To create a custom service, implement the `BackgroundService` trait:

```rust
#[derive(Clone)]
pub struct MyCustomService {
    // ... fields
}

impl BackgroundService for MyCustomService {
    type Config = MyConfig;
    type Error = MyError;

    fn service_name(&self) -> &'static str {
        "my-custom-service"
    }

    async fn initialize(&mut self, config: Self::Config) -> Result<(), Self::Error> {
        // Initialize your service
        Ok(())
    }

    async fn run(&mut self, mut shutdown_rx: broadcast::Receiver<()>) -> Result<(), Self::Error> {
        // Main service loop
        tokio::select! {
            _ = self.do_work() => Ok(()),
            _ = shutdown_rx.recv() => {
                info!("Shutting down gracefully");
                Ok(())
            }
        }
    }

    // ... other trait methods
}

impl ClonableService for MyCustomService {}
```

### Multiple Service Coordination

```rust
// Start multiple services
let mut http_manager = DefaultServiceManager::new(http_service, config.clone(), false);
let mut cluster_manager = DefaultServiceManager::new(cluster_service, config.clone(), false);

// Start in dependency order
http_manager.start().await?;
cluster_manager.start().await?;

// Monitor all services
let http_health = http_manager.health().await;
let cluster_health = cluster_manager.health().await;

// Stop in reverse order
cluster_manager.stop().await?;
http_manager.stop().await?;
```

### Health Monitoring

```rust
// Continuous health monitoring
let mut manager = DefaultServiceManager::new(service, config, false);
manager.start().await?;

loop {
    match manager.health().await {
        ServiceHealth::Healthy => {
            println!("✅ Service is healthy");
        }
        ServiceHealth::Unhealthy { reason } => {
            println!("❌ Service unhealthy: {}", reason);
            // Consider restarting
            manager.restart().await?;
        }
        ServiceHealth::Unknown => {
            println!("❓ Health status unknown");
        }
    }
    
    tokio::time::sleep(Duration::from_secs(30)).await;
}
```

## Error Handling

### Service Errors

The system provides comprehensive error handling:

```rust
match manager.start().await {
    Ok(_) => println!("Service started successfully"),
    Err(ServiceError::AlreadyRunning(name)) => {
        println!("Service '{}' is already running", name);
    }
    Err(ServiceError::ConfigurationError(msg)) => {
        println!("Configuration error: {}", msg);
    }
    Err(ServiceError::InitializationFailed(msg)) => {
        println!("Initialization failed: {}", msg);
    }
    Err(ServiceError::ShutdownTimeout) => {
        println!("Service shutdown timed out");
    }
    Err(e) => println!("Unexpected error: {}", e),
}
```

### Configuration Validation

Validate configurations before starting services:

```rust
// Validate before creating service
if let Err(e) = HttpServerService::validate_config(&config) {
    eprintln!("Invalid configuration: {}", e);
    return Err(e.into());
}

let service = HttpServerServiceBuilder::new().build();
let mut manager = DefaultServiceManager::new(service, config, false);
manager.start().await?;
```

## Testing

### Unit Tests

```rust
#[tokio::test]
async fn test_service_lifecycle() {
    let config = Config::default();
    let service = HttpServerServiceBuilder::new()
        .foreground_mode(false)
        .build();
    let mut manager = DefaultServiceManager::new(service, config, false);

    assert!(!manager.is_running());
    
    // Test would require mock server for full integration
    assert_eq!(manager.service_name(), "http-server");
}
```

### Integration Tests

For full integration testing, the `examples/background_service_examples.rs` file provides comprehensive examples that can be adapted for your testing needs.

## Best Practices

### 1. Service Lifecycle Management

- Always check `is_running()` before starting a service
- Use `restart()` instead of manual stop/start sequences
- Handle `ServiceError::AlreadyRunning` gracefully
- Always call `stop()` in cleanup code

### 2. Configuration Management

- Validate configurations using `validate_config()` before service creation
- Use builder patterns for clean service construction
- Leverage environment variable overrides for deployment flexibility

### 3. Logging and Monitoring

- Use the unified logging system for consistent log formatting
- Implement regular health checks for long-running services
- Monitor service uptime and status in production deployments

### 4. Error Handling

- Handle all `ServiceError` variants explicitly
- Use timeout-based operations for network-dependent services
- Implement retry logic for transient failures

### 5. Resource Management

- Services automatically clean up resources on shutdown
- Trust Rust's ownership system for memory management
- Use structured concurrency patterns with Tokio

## Comparison with Legacy System

| Feature | Legacy Daemon | New Background Service |
|---------|---------------|------------------------|
| **Safety** | ⚠️ Some unsafe code | ✅ 100% safe Rust |
| **Process Model** | 2+ processes | 1 process |
| **Memory Usage** | Higher | Lower |
| **Error Handling** | Complex | Simple & comprehensive |
| **Testing** | Difficult | Easy with mocks |
| **Platform Support** | Platform-specific issues | Unified across platforms |
| **Maintenance** | Complex | Simple |
| **Health Monitoring** | Basic | Advanced |
| **Configuration** | Manual validation | Automatic validation |
| **Logging** | Inconsistent | Unified system |

## Troubleshooting

### Common Issues

1. **Service won't start**
   - Check configuration validation errors
   - Verify ports are not in use
   - Check log directory permissions

2. **Service hangs on shutdown**
   - Default timeout is 30 seconds
   - Check for blocking operations in service code
   - Review shutdown signal handling

3. **Health checks fail**
   - Verify service endpoints are accessible
   - Check network configuration
   - Review health check implementation

4. **Configuration errors**
   - Use `validate_config()` to check configuration
   - Review configuration file syntax
   - Check environment variable overrides

### Debug Mode

Enable debug logging for detailed troubleshooting:

```bash
export VOICE_CLI_LOG_LEVEL="debug"
voice-cli server start
```

Or in configuration:

```yaml
logging:
  level: "debug"
```

## Future Enhancements

The new background service abstraction is designed to be extensible. Planned enhancements include:

- **Metrics Collection**: Built-in Prometheus metrics
- **Service Discovery**: Automatic service registration
- **Circuit Breakers**: Advanced failure handling
- **Blue-Green Deployments**: Zero-downtime updates
- **Container Integration**: Native Docker/Kubernetes support

## Conclusion

The new unified background service abstraction provides a modern, safe, and consistent approach to managing background services in voice-cli. It eliminates the complexity and safety concerns of the previous daemon system while providing enhanced functionality and better developer experience.

By following this guide, you can effectively migrate from the legacy daemon system and take advantage of the improved architecture for reliable, production-ready service management.

For more examples and detailed usage patterns, refer to:
- `examples/background_service_examples.rs` - Comprehensive examples
- `src/cli/unified_handlers.rs` - CLI integration patterns
- `src/daemon/services/` - Service implementation details