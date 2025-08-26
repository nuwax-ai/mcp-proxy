use crate::config::{ConfigTemplateGenerator, ServiceType};
use crate::daemon::{HttpServerService, DefaultServiceManager};
use crate::models::Config;
use crate::VoiceCliError;
use std::path::PathBuf;
use tracing::{info, warn};

/// Initialize server configuration
pub async fn handle_server_init(config_path: Option<PathBuf>, force: bool) -> crate::Result<()> {
    let output_path = config_path.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("server-config.yml")
    });

    // 检查文件是否已存在
    if output_path.exists() && !force {
        println!("❌ Configuration file already exists: {:?}", output_path);
        println!("💡 Use --force to overwrite, or specify a different path with --config");
        return Ok(());
    }

    // 生成配置文件
    ConfigTemplateGenerator::generate_config_file(ServiceType::Server, &output_path)?;

    println!("✅ Server configuration initialized: {:?}", output_path);
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli server run --config {:?}", output_path);

    Ok(())
}

/// Run server in foreground mode (direct HTTP server)
pub async fn handle_server_run(config: &Config) -> crate::Result<()> {
    info!("Starting voice-cli server in foreground mode...");

    // Initialize logging
    crate::utils::init_logging(config)?;

    // Start the cluster-aware HTTP server
    let server = crate::server::create_cluster_aware_server_with_shutdown(config.clone()).await?;

    if config.cluster.enabled {
        info!(
            "Cluster-aware server running on {}:{}",
            config.server.host, config.server.port
        );
        info!("Cluster node ID: {}", config.cluster.node_id);
        info!("gRPC port: {}", config.cluster.grpc_port);
    } else {
        info!(
            "Single-node server running on {}:{}",
            config.server.host, config.server.port
        );
    }
    info!("Press Ctrl+C to stop the server");

    // Run server with graceful shutdown
    server
        .await
        .map_err(|e| VoiceCliError::Config(format!("Server error: {}", e)))?;

    Ok(())
}

/// Start server as daemon (background process)
pub async fn handle_server_start(config: &Config) -> crate::Result<()> {
    let service = HttpServerService::new(false); // false = background mode
    let mut manager = DefaultServiceManager::new(service, config.clone(), false);
    manager.start().await
        .map_err(|e| VoiceCliError::Daemon(e.to_string()))?;
    info!("Server daemon started successfully");
    Ok(())
}

/// Stop daemon server
pub async fn handle_server_stop(config: &Config) -> crate::Result<()> {
    let service = HttpServerService::new(false); // false = background mode
    let mut manager = DefaultServiceManager::new(service, config.clone(), false);
    manager.stop().await
        .map_err(|e| VoiceCliError::Daemon(e.to_string()))?;
    info!("Server daemon stopped successfully");
    Ok(())
}

/// Restart daemon server
pub async fn handle_server_restart(config: &Config) -> crate::Result<()> {
    let service = HttpServerService::new(false); // false = background mode
    let mut manager = DefaultServiceManager::new(service, config.clone(), false);
    manager.restart().await
        .map_err(|e| VoiceCliError::Daemon(e.to_string()))?;
    info!("Server daemon restarted successfully");
    Ok(())
}

/// Get daemon server status
pub async fn handle_server_status(config: &Config) -> crate::Result<()> {
    let service = HttpServerService::new(false); // false = background mode
    let manager = DefaultServiceManager::new(service, config.clone(), false);
    
    if manager.is_running() {
        info!("Server is running");
        
        match manager.health().await {
            crate::daemon::ServiceHealth::Healthy => {
                info!("Server health: healthy");
            }
            crate::daemon::ServiceHealth::Unhealthy { reason } => {
                warn!("Server health: unhealthy - {}", reason);
            }
            crate::daemon::ServiceHealth::Unknown => {
                warn!("Server health: unknown");
            }
        }
        
        if let Some(uptime) = manager.uptime() {
            info!("Uptime: {:?}", uptime);
        }
    } else {
        info!("Server is not running");
    }
    
    Ok(())
}

/// Internal daemon serve command (called by daemon process)
pub async fn handle_daemon_serve(config: &Config) -> crate::Result<()> {
    info!("Starting daemon HTTP server...");

    // Initialize logging for daemon
    crate::utils::init_logging(config)?;

    // Start the cluster-aware HTTP server
    let server = crate::server::create_cluster_aware_server(config.clone()).await?;

    if config.cluster.enabled {
        info!(
            "Daemon cluster-aware server running on {}:{}",
            config.server.host, config.server.port
        );
        info!("Cluster node ID: {}", config.cluster.node_id);
    } else {
        info!(
            "Daemon single-node server running on {}:{}",
            config.server.host, config.server.port
        );
    }

    // Run server (this will block until shutdown)
    server
        .await
        .map_err(|e| VoiceCliError::Config(format!("Daemon server error: {}", e)))?;

    Ok(())
}
