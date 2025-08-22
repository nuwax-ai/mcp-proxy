use crate::daemon::{DaemonService, DaemonStatus};
use crate::models::Config;
use crate::VoiceCliError;
use tracing::{info, warn};

/// Run server in foreground mode (direct HTTP server)
pub async fn handle_server_run(config: &Config) -> crate::Result<()> {
    info!("Starting voice-cli server in foreground mode...");
    
    // Initialize logging
    crate::utils::init_logging(config)?;
    
    // Start the HTTP server directly
    let server = crate::server::create_server(config.clone()).await?;
    
    info!("Server running on {}:{}", config.server.host, config.server.port);
    info!("Press Ctrl+C to stop the server");
    
    // Run server with graceful shutdown
    server.await.map_err(|e| VoiceCliError::Config(format!("Server error: {}", e)))?;
    
    Ok(())
}

/// Start server as daemon (background process)
pub async fn handle_server_start(config: &Config) -> crate::Result<()> {
    let daemon_service = DaemonService::new(config.clone());
    daemon_service.start_daemon().await
}

/// Stop daemon server
pub async fn handle_server_stop(config: &Config) -> crate::Result<()> {
    let daemon_service = DaemonService::new(config.clone());
    daemon_service.stop_daemon().await
}

/// Restart daemon server
pub async fn handle_server_restart(config: &Config) -> crate::Result<()> {
    let daemon_service = DaemonService::new(config.clone());
    daemon_service.restart_daemon().await
}

/// Get daemon server status
pub async fn handle_server_status(config: &Config) -> crate::Result<()> {
    let daemon_service = DaemonService::new(config.clone());
    
    match daemon_service.get_status().await? {
        DaemonStatus::Running { pid, health } => {
            info!("Server is running with PID: {}", pid);
            
            if let Some(health_info) = health {
                info!("Server health: {}", health_info.status);
                info!("Uptime: {} seconds", health_info.uptime);
                info!("Models loaded: {:?}", health_info.models_loaded);
                info!("Version: {}", health_info.version);
            } else {
                warn!("Server is running but health check failed");
            }
        }
        DaemonStatus::Stopped => {
            info!("Server is not running");
        }
    }
    
    Ok(())
}

/// Internal daemon serve command (called by daemon process)
pub async fn handle_daemon_serve(config: &Config) -> crate::Result<()> {
    info!("Starting daemon HTTP server...");
    
    // Initialize logging for daemon
    crate::utils::init_logging(config)?;
    
    // Start the HTTP server
    let server = crate::server::create_server(config.clone()).await?;
    
    info!("Daemon HTTP server running on {}:{}", config.server.host, config.server.port);
    
    // Run server (this will block until shutdown)
    server.await.map_err(|e| VoiceCliError::Config(format!("Daemon server error: {}", e)))?;
    
    Ok(())
}