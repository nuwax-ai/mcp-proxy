pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod http_tracing;
pub mod middleware_config;
pub mod app_state;

use crate::models::Config;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};




async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Signal received, starting graceful shutdown");
}

async fn shutdown_signal_with_broadcast(shutdown_tx: broadcast::Sender<()>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("Signal received, starting graceful shutdown");
    let _ = shutdown_tx.send(());
}

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
    crate::config::ConfigTemplateGenerator::generate_config_file(crate::config::ServiceType::Server, &output_path)?;

    println!("✅ Server configuration initialized: {:?}", output_path);
    println!("📝 Edit the configuration file and run:");
    println!("   voice-cli server run --config {:?}", output_path);

    Ok(())
}

/// Run server in foreground mode (direct HTTP server)
pub async fn handle_server_run(config: &Config) -> crate::Result<()> {
    info!("Starting voice-cli server in foreground mode...");

    // Initialize logging - keep the guard alive for the duration of the process
    info!("About to initialize logging...");
    crate::utils::init_logging(config)?;
    info!("Logging initialized successfully");

    let config_arc = Arc::new(config.clone());
    let app_state = handlers::AppState::new(config_arc.clone()).await?;
    let mut app = routes::create_routes_with_state(app_state.clone()).await?;
    
    // Clone app_state for use in monitor
    let app_state_for_monitor = app_state.clone();
    
    // Create shutdown channel for monitor task
    let (shutdown_tx, _) = broadcast::channel(1);
    let mut shutdown_rx = shutdown_tx.subscribe();
    
    // 添加 storage 作为 Extension
    app = app.layer(axum::Extension(app_state.apalis_storage.clone()));

    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    info!("TCP listener created successfully: {:?}", listener.local_addr());
    info!("Starting axum server...");

    let http = async {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal_with_broadcast(shutdown_tx))
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
        
        info!("Axum server completed, performing graceful shutdown...");
        
        // Perform graceful shutdown of application state
        app_state.shutdown().await;
        
        // Perform global cleanup operations
        crate::utils::perform_shutdown_cleanup().await;
        
        info!("Graceful shutdown completed with result: {:?}", result);
        result
    };

    let monitor = async {
        // Wait for shutdown signal
        let _ = shutdown_rx.recv().await;
        info!("Monitor task received shutdown signal, stopping Apalis manager...");
        
        // Gracefully shutdown the Apalis manager
        if let Err(e) = app_state_for_monitor.lock_free_apalis_manager.shutdown().await {
            warn!("Failed to shutdown Apalis manager gracefully: {}", e);
        }
        
        Ok::<(), std::io::Error>(())
    };

    let _res = tokio::join!(http, monitor);

    Ok(())
}
