pub mod handlers;
pub mod middleware;
pub mod routes;
pub mod cluster_handlers;
pub mod cluster_routes;

use crate::models::Config;
use std::net::SocketAddr;
use tracing::info;

/// Create server with cluster awareness
pub async fn create_cluster_aware_server(config: Config) -> crate::Result<impl std::future::Future<Output = Result<(), std::io::Error>>> {
    let app = cluster_routes::create_cluster_routes(config.clone()).await?;
    
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Cluster-aware server listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    Ok(async move {
        axum::serve(listener, app).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    })
}

/// Create cluster-aware server with graceful shutdown
pub async fn create_cluster_aware_server_with_shutdown(
    config: Config,
) -> crate::Result<impl std::future::Future<Output = Result<(), std::io::Error>>> {
    let app = cluster_routes::create_cluster_routes(config.clone()).await?;
    
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Cluster-aware server with graceful shutdown listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    Ok(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    })
}

pub async fn create_server(config: Config) -> crate::Result<impl std::future::Future<Output = Result<(), std::io::Error>>> {
    let app = routes::create_routes(config.clone()).await?;
    
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Server listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    Ok(async move {
        axum::serve(listener, app).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    })
}

pub async fn create_server_with_graceful_shutdown(
    config: Config,
) -> crate::Result<impl std::future::Future<Output = Result<(), std::io::Error>>> {
    let app = routes::create_routes(config.clone()).await?;
    
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.port));
    info!("Server listening on {}", addr);
    
    let listener = tokio::net::TcpListener::bind(&addr).await
        .map_err(|e| crate::VoiceCliError::Config(format!("Failed to bind to address: {}", e)))?;
    
    Ok(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    })
}

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