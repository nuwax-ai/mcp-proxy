//! Unified CLI Handlers
//! 
//! This module provides CLI handlers that use the new unified background service abstraction.
//! These handlers replace the legacy daemon-based implementations with modern, safe, and consistent approaches.

use crate::daemon::{
    DefaultServiceManager, HttpServerServiceBuilder, ClusterNodeServiceBuilder, LoadBalancerServiceBuilder,
    ServiceHealth, ServiceStatus
};
use crate::models::Config;
use crate::VoiceCliError;
use tracing::{info, error, warn};

/// Unified server command handlers using the new background service abstraction
pub mod server {
    use super::*;

    /// Run server in foreground mode using unified service abstraction
    pub async fn handle_run(config: &Config) -> crate::Result<()> {
        info!("Starting HTTP server in foreground mode (unified)");

        // Create HTTP server service in foreground mode
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(true)
            .build();

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), true);

        // Start and wait for completion (foreground mode blocks)
        match manager.start().await {
            Ok(_) => {
                info!("HTTP server started successfully");
                
                // In foreground mode, we need to wait for shutdown signals
                // The service will handle this internally
                tokio::signal::ctrl_c().await
                    .map_err(|e| VoiceCliError::Config(format!("Signal handling error: {}", e)))?;
                
                info!("Received shutdown signal, stopping server...");
                manager.stop().await?;
                
                Ok(())
            }
            Err(e) => {
                error!("Failed to start HTTP server: {}", e);
                Err(VoiceCliError::Daemon(format!("Server startup failed: {}", e)))
            }
        }
    }

    /// Start server in background mode using unified service abstraction
    pub async fn handle_start(config: &Config) -> crate::Result<()> {
        info!("Starting HTTP server in background mode (unified)");

        // Create HTTP server service in background mode
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        // Check if already running
        if manager.is_running() {
            return Err(VoiceCliError::Daemon(
                "HTTP server is already running".to_string()
            ));
        }

        // Start the service
        manager.start().await?;

        // Wait a moment to ensure startup
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        // Check status
        match manager.status() {
            ServiceStatus::Running => {
                match manager.health().await {
                    ServiceHealth::Healthy => {
                        info!("HTTP server started successfully and is healthy");
                        println!("✅ HTTP server is running at http://{}:{}", 
                                config.server.host, config.server.port);
                        if config.cluster.enabled {
                            println!("🔗 Cluster mode enabled - Node ID: {}", config.cluster.node_id);
                            println!("📡 gRPC cluster port: {}", config.cluster.grpc_port);
                        }
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        warn!("HTTP server started but is unhealthy: {}", reason);
                        println!("⚠️  HTTP server is running but may have issues: {}", reason);
                    }
                    ServiceHealth::Unknown => {
                        warn!("HTTP server health status unknown");
                        println!("❓ HTTP server is running but health status is unknown");
                    }
                }
            }
            ServiceStatus::Failed { error } => {
                return Err(VoiceCliError::Daemon(format!("Server startup failed: {}", error)));
            }
            _ => {
                return Err(VoiceCliError::Daemon("Server failed to start properly".to_string()));
            }
        }

        Ok(())
    }

    /// Stop server using unified service abstraction
    pub async fn handle_stop(config: &Config) -> crate::Result<()> {
        info!("Stopping HTTP server (unified)");

        // Create service manager to check status and stop if needed
        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        if !manager.is_running() {
            info!("HTTP server is not running");
            println!("ℹ️  HTTP server is already stopped");
            return Ok(());
        }

        // Stop the service
        manager.stop().await?;

        info!("HTTP server stopped successfully");
        println!("✅ HTTP server has been stopped");

        Ok(())
    }

    /// Restart server using unified service abstraction
    pub async fn handle_restart(config: &Config) -> crate::Result<()> {
        info!("Restarting HTTP server (unified)");

        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        println!("🔄 Restarting HTTP server...");
        manager.restart().await?;

        // Wait for restart to complete
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        // Check final status
        match manager.health().await {
            ServiceHealth::Healthy => {
                info!("HTTP server restarted successfully");
                println!("✅ HTTP server restarted and is healthy");
                println!("🌐 Server available at http://{}:{}", config.server.host, config.server.port);
            }
            ServiceHealth::Unhealthy { reason } => {
                warn!("HTTP server restarted but is unhealthy: {}", reason);
                println!("⚠️  HTTP server restarted but may have issues: {}", reason);
            }
            ServiceHealth::Unknown => {
                warn!("HTTP server health status unknown after restart");
                println!("❓ HTTP server restarted but health status is unknown");
            }
        }

        Ok(())
    }

    /// Get server status using unified service abstraction
    pub async fn handle_status(config: &Config) -> crate::Result<()> {
        info!("Checking HTTP server status (unified)");

        let service = HttpServerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let manager = DefaultServiceManager::new(service, config.clone(), false);

        let status = manager.status();
        let health = manager.health().await;

        println!("=== HTTP Server Status ===");
        
        match status {
            ServiceStatus::Running => {
                println!("Status: 🟢 RUNNING");
                
                if let Some(uptime) = manager.uptime() {
                    println!("Uptime: {:?}", uptime);
                }
                
                match health {
                    ServiceHealth::Healthy => {
                        println!("Health: ✅ HEALTHY");
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        println!("Health: ❌ UNHEALTHY");
                        println!("Reason: {}", reason);
                    }
                    ServiceHealth::Unknown => {
                        println!("Health: ❓ UNKNOWN");
                    }
                }
                
                println!("Endpoint: http://{}:{}", config.server.host, config.server.port);
                
                if config.cluster.enabled {
                    println!("Cluster: ENABLED");
                    println!("Node ID: {}", config.cluster.node_id);
                    println!("gRPC Port: {}", config.cluster.grpc_port);
                }
            }
            ServiceStatus::Starting => {
                println!("Status: 🟡 STARTING");
            }
            ServiceStatus::Stopping => {
                println!("Status: 🟡 STOPPING");
            }
            ServiceStatus::Stopped => {
                println!("Status: 🔴 STOPPED");
                println!("Use 'voice-cli server start' to start the server");
            }
            ServiceStatus::Failed { error } => {
                println!("Status: ❌ FAILED");
                println!("Error: {}", error);
            }
        }
        
        println!("============================");

        Ok(())
    }
}

/// Unified cluster command handlers using the new background service abstraction
pub mod cluster {
    use super::*;

    /// Run cluster node in foreground mode using unified service abstraction
    pub async fn handle_run(config: &Config, node_id: Option<String>) -> crate::Result<()> {
        let node_id = node_id.unwrap_or_else(|| config.cluster.node_id.clone());
        info!("Starting cluster node '{}' in foreground mode (unified)", node_id);

        // Create cluster node service in foreground mode
        let service = ClusterNodeServiceBuilder::new()
            .node_id(node_id.clone())
            .foreground_mode(true)
            .build()?;

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), true);

        // Start and wait for completion (foreground mode blocks)
        match manager.start().await {
            Ok(_) => {
                info!("Cluster node '{}' started successfully", node_id);
                
                // In foreground mode, we need to wait for shutdown signals
                tokio::signal::ctrl_c().await
                    .map_err(|e| VoiceCliError::Config(format!("Signal handling error: {}", e)))?;
                
                info!("Received shutdown signal, stopping cluster node...");
                manager.stop().await?;
                
                Ok(())
            }
            Err(e) => {
                error!("Failed to start cluster node '{}': {}", node_id, e);
                Err(VoiceCliError::Daemon(format!("Cluster node startup failed: {}", e)))
            }
        }
    }

    /// Start cluster node in background mode using unified service abstraction
    pub async fn handle_start(config: &Config, node_id: Option<String>) -> crate::Result<()> {
        let node_id = node_id.unwrap_or_else(|| config.cluster.node_id.clone());
        info!("Starting cluster node '{}' in background mode (unified)", node_id);

        // Create cluster node service in background mode
        let service = ClusterNodeServiceBuilder::new()
            .node_id(node_id.clone())
            .foreground_mode(false)
            .build()?;

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        // Check if already running
        if manager.is_running() {
            return Err(VoiceCliError::Daemon(
                format!("Cluster node '{}' is already running", node_id)
            ));
        }

        // Start the service
        manager.start().await?;

        // Wait a moment to ensure startup
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        // Check status
        match manager.status() {
            ServiceStatus::Running => {
                match manager.health().await {
                    ServiceHealth::Healthy => {
                        info!("Cluster node '{}' started successfully and is healthy", node_id);
                        println!("✅ Cluster node '{}' is running", node_id);
                        println!("📡 gRPC cluster: {}:{}", config.cluster.bind_address, config.cluster.grpc_port);
                        println!("🌐 HTTP API: {}:{}", config.cluster.bind_address, config.cluster.http_port);
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        warn!("Cluster node '{}' started but is unhealthy: {}", node_id, reason);
                        println!("⚠️  Cluster node '{}' is running but may have issues: {}", node_id, reason);
                    }
                    ServiceHealth::Unknown => {
                        warn!("Cluster node '{}' health status unknown", node_id);
                        println!("❓ Cluster node '{}' is running but health status is unknown", node_id);
                    }
                }
            }
            ServiceStatus::Failed { error } => {
                return Err(VoiceCliError::Daemon(format!("Cluster node startup failed: {}", error)));
            }
            _ => {
                return Err(VoiceCliError::Daemon("Cluster node failed to start properly".to_string()));
            }
        }

        Ok(())
    }

    /// Stop cluster node using unified service abstraction
    pub async fn handle_stop(config: &Config, node_id: Option<String>) -> crate::Result<()> {
        let node_id = node_id.unwrap_or_else(|| config.cluster.node_id.clone());
        info!("Stopping cluster node '{}' (unified)", node_id);

        // Create service manager to check status and stop if needed
        let service = ClusterNodeServiceBuilder::new()
            .node_id(node_id.clone())
            .foreground_mode(false)
            .build()?;

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        if !manager.is_running() {
            info!("Cluster node '{}' is not running", node_id);
            println!("ℹ️  Cluster node '{}' is already stopped", node_id);
            return Ok(());
        }

        // Stop the service
        manager.stop().await?;

        info!("Cluster node '{}' stopped successfully", node_id);
        println!("✅ Cluster node '{}' has been stopped", node_id);

        Ok(())
    }

    /// Restart cluster node using unified service abstraction
    pub async fn handle_restart(config: &Config, node_id: Option<String>) -> crate::Result<()> {
        let node_id = node_id.unwrap_or_else(|| config.cluster.node_id.clone());
        info!("Restarting cluster node '{}' (unified)", node_id);

        let service = ClusterNodeServiceBuilder::new()
            .node_id(node_id.clone())
            .foreground_mode(false)
            .build()?;

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        println!("🔄 Restarting cluster node '{}'...", node_id);
        manager.restart().await?;

        // Wait for restart to complete
        tokio::time::sleep(std::time::Duration::from_millis(3000)).await;

        // Check final status
        match manager.health().await {
            ServiceHealth::Healthy => {
                info!("Cluster node '{}' restarted successfully", node_id);
                println!("✅ Cluster node '{}' restarted and is healthy", node_id);
            }
            ServiceHealth::Unhealthy { reason } => {
                warn!("Cluster node '{}' restarted but is unhealthy: {}", node_id, reason);
                println!("⚠️  Cluster node '{}' restarted but may have issues: {}", node_id, reason);
            }
            ServiceHealth::Unknown => {
                warn!("Cluster node '{}' health status unknown after restart", node_id);
                println!("❓ Cluster node '{}' restarted but health status is unknown", node_id);
            }
        }

        Ok(())
    }

    /// Get cluster node status using unified service abstraction
    pub async fn handle_status(config: &Config, node_id: Option<String>) -> crate::Result<()> {
        let node_id = node_id.unwrap_or_else(|| config.cluster.node_id.clone());
        info!("Checking cluster node '{}' status (unified)", node_id);

        let service = ClusterNodeServiceBuilder::new()
            .node_id(node_id.clone())
            .foreground_mode(false)
            .build()?;

        let manager = DefaultServiceManager::new(service, config.clone(), false);

        let status = manager.status();
        let health = manager.health().await;

        println!("=== Cluster Node '{}' Status ===", node_id);
        
        match status {
            ServiceStatus::Running => {
                println!("Status: 🟢 RUNNING");
                
                if let Some(uptime) = manager.uptime() {
                    println!("Uptime: {:?}", uptime);
                }
                
                match health {
                    ServiceHealth::Healthy => {
                        println!("Health: ✅ HEALTHY");
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        println!("Health: ❌ UNHEALTHY");
                        println!("Reason: {}", reason);
                    }
                    ServiceHealth::Unknown => {
                        println!("Health: ❓ UNKNOWN");
                    }
                }
                
                println!("gRPC Cluster: {}:{}", config.cluster.bind_address, config.cluster.grpc_port);
                println!("HTTP API: {}:{}", config.cluster.bind_address, config.cluster.http_port);
                println!("Leader can process tasks: {}", config.cluster.leader_can_process_tasks);
            }
            ServiceStatus::Starting => {
                println!("Status: 🟡 STARTING");
            }
            ServiceStatus::Stopping => {
                println!("Status: 🟡 STOPPING");
            }
            ServiceStatus::Stopped => {
                println!("Status: 🔴 STOPPED");
                println!("Use 'voice-cli cluster start' to start the node");
            }
            ServiceStatus::Failed { error } => {
                println!("Status: ❌ FAILED");
                println!("Error: {}", error);
            }
        }
        
        println!("=====================================");

        Ok(())
    }
}

/// Unified load balancer command handlers using the new background service abstraction
pub mod load_balancer {
    use super::*;

    /// Run load balancer in foreground mode using unified service abstraction
    pub async fn handle_run(config: &Config) -> crate::Result<()> {
        info!("Starting load balancer in foreground mode (unified)");

        // Create load balancer service in foreground mode
        let service = LoadBalancerServiceBuilder::new()
            .foreground_mode(true)
            .build();

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), true);

        // Start and wait for completion (foreground mode blocks)
        match manager.start().await {
            Ok(_) => {
                info!("Load balancer started successfully");
                
                // In foreground mode, we need to wait for shutdown signals
                tokio::signal::ctrl_c().await
                    .map_err(|e| VoiceCliError::Config(format!("Signal handling error: {}", e)))?;
                
                info!("Received shutdown signal, stopping load balancer...");
                manager.stop().await?;
                
                Ok(())
            }
            Err(e) => {
                error!("Failed to start load balancer: {}", e);
                Err(VoiceCliError::Daemon(format!("Load balancer startup failed: {}", e)))
            }
        }
    }

    /// Start load balancer in background mode using unified service abstraction
    pub async fn handle_start(config: &Config) -> crate::Result<()> {
        info!("Starting load balancer in background mode (unified)");

        // Create load balancer service in background mode
        let service = LoadBalancerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        // Create service manager
        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        // Check if already running
        if manager.is_running() {
            return Err(VoiceCliError::Daemon(
                "Load balancer is already running".to_string()
            ));
        }

        // Start the service
        manager.start().await?;

        // Wait a moment to ensure startup
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        // Check status
        match manager.status() {
            ServiceStatus::Running => {
                match manager.health().await {
                    ServiceHealth::Healthy => {
                        info!("Load balancer started successfully and is healthy");
                        println!("✅ Load balancer is running at http://{}:{}", 
                                config.load_balancer.bind_address, config.load_balancer.port);
                        println!("🔍 Health check interval: {}s", config.load_balancer.health_check_interval);
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        warn!("Load balancer started but is unhealthy: {}", reason);
                        println!("⚠️  Load balancer is running but may have issues: {}", reason);
                    }
                    ServiceHealth::Unknown => {
                        warn!("Load balancer health status unknown");
                        println!("❓ Load balancer is running but health status is unknown");
                    }
                }
            }
            ServiceStatus::Failed { error } => {
                return Err(VoiceCliError::Daemon(format!("Load balancer startup failed: {}", error)));
            }
            _ => {
                return Err(VoiceCliError::Daemon("Load balancer failed to start properly".to_string()));
            }
        }

        Ok(())
    }

    /// Stop load balancer using unified service abstraction
    pub async fn handle_stop(config: &Config) -> crate::Result<()> {
        info!("Stopping load balancer (unified)");

        // Create service manager to check status and stop if needed
        let service = LoadBalancerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        if !manager.is_running() {
            info!("Load balancer is not running");
            println!("ℹ️  Load balancer is already stopped");
            return Ok(());
        }

        // Stop the service
        manager.stop().await?;

        info!("Load balancer stopped successfully");
        println!("✅ Load balancer has been stopped");

        Ok(())
    }

    /// Restart load balancer using unified service abstraction
    pub async fn handle_restart(config: &Config) -> crate::Result<()> {
        info!("Restarting load balancer (unified)");

        let service = LoadBalancerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let mut manager = DefaultServiceManager::new(service, config.clone(), false);

        println!("🔄 Restarting load balancer...");
        manager.restart().await?;

        // Wait for restart to complete
        tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

        // Check final status
        match manager.health().await {
            ServiceHealth::Healthy => {
                info!("Load balancer restarted successfully");
                println!("✅ Load balancer restarted and is healthy");
                println!("🌐 Load balancer available at http://{}:{}", 
                        config.load_balancer.bind_address, config.load_balancer.port);
            }
            ServiceHealth::Unhealthy { reason } => {
                warn!("Load balancer restarted but is unhealthy: {}", reason);
                println!("⚠️  Load balancer restarted but may have issues: {}", reason);
            }
            ServiceHealth::Unknown => {
                warn!("Load balancer health status unknown after restart");
                println!("❓ Load balancer restarted but health status is unknown");
            }
        }

        Ok(())
    }

    /// Get load balancer status using unified service abstraction
    pub async fn handle_status(config: &Config) -> crate::Result<()> {
        info!("Checking load balancer status (unified)");

        let service = LoadBalancerServiceBuilder::new()
            .foreground_mode(false)
            .build();

        let manager = DefaultServiceManager::new(service, config.clone(), false);

        let status = manager.status();
        let health = manager.health().await;

        println!("=== Load Balancer Status ===");
        
        match status {
            ServiceStatus::Running => {
                println!("Status: 🟢 RUNNING");
                
                if let Some(uptime) = manager.uptime() {
                    println!("Uptime: {:?}", uptime);
                }
                
                match health {
                    ServiceHealth::Healthy => {
                        println!("Health: ✅ HEALTHY");
                    }
                    ServiceHealth::Unhealthy { reason } => {
                        println!("Health: ❌ UNHEALTHY");
                        println!("Reason: {}", reason);
                    }
                    ServiceHealth::Unknown => {
                        println!("Health: ❓ UNKNOWN");
                    }
                }
                
                println!("Endpoint: http://{}:{}", config.load_balancer.bind_address, config.load_balancer.port);
                println!("Health check interval: {}s", config.load_balancer.health_check_interval);
                println!("Health check timeout: {}s", config.load_balancer.health_check_timeout);
            }
            ServiceStatus::Starting => {
                println!("Status: 🟡 STARTING");
            }
            ServiceStatus::Stopping => {
                println!("Status: 🟡 STOPPING");
            }
            ServiceStatus::Stopped => {
                println!("Status: 🔴 STOPPED");
                println!("Use 'voice-cli lb start' to start the load balancer");
            }
            ServiceStatus::Failed { error } => {
                println!("Status: ❌ FAILED");
                println!("Error: {}", error);
            }
        }
        
        println!("==============================");

        Ok(())
    }
}