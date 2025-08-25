use crate::config::{ConfigTemplateGenerator, ServiceType};
use crate::daemon::{DaemonService, DaemonStatus};
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
