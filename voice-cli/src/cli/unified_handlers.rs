//! Unified CLI Handlers
//! 
//! This module provides CLI handlers that use the new unified background service abstraction.
//! These handlers replace the legacy daemon-based implementations with modern, safe, and consistent approaches.

use crate::daemon::{
    DefaultServiceManager, HttpServerServiceBuilder,
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

