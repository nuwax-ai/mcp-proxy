//! Shared signal handling utilities
//!
//! This module provides unified signal handling for background services,
//! eliminating duplicate signal handling code across different services.

use tokio::signal;
use tracing::{debug, error, info};

/// Create a unified shutdown signal handler that listens to multiple sources
///
/// This function provides consistent signal handling across all services:
/// - Ctrl+C (SIGINT)
/// - SIGTERM (on Unix platforms)
/// - Manual shutdown signals
///
/// # Example
/// ```rust,no_run
/// use voice_cli::utils::signal_handling::create_shutdown_signal;
/// use tracing::info;
///
/// # async fn example() {
/// let shutdown_signal = create_shutdown_signal();
/// tokio::select! {
///     _ = async { /* service_work */ } => {},
///     _ = shutdown_signal => {
///         info!("Received shutdown signal, stopping service");
///     }
/// }
/// # }
/// ```
pub async fn create_shutdown_signal() {
    handle_system_signals().await
}

/// Handle system signals for graceful shutdown
///
/// This provides the core signal handling logic used by all services.
/// Separated into its own function for reusability and testing.
pub async fn handle_system_signals() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failed to listen for Ctrl+C: {}", e);
        } else {
            info!("Received Ctrl+C signal");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
                info!("Received SIGTERM signal");
            }
            Err(e) => {
                error!("Failed to listen for SIGTERM: {}", e);
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => debug!("Ctrl+C signal handled"),
        _ = terminate => debug!("SIGTERM signal handled"),
    }
}

/// Create a combined shutdown signal that listens to both system signals and manual channels
///
/// This is useful for services that need to respond to both system signals (Ctrl+C, SIGTERM)
/// and programmatic shutdown requests via channels.
///
/// # Arguments
/// * `manual_shutdown` - A future that completes when manual shutdown is requested
///
/// # Example
/// ```rust,no_run
/// use voice_cli::utils::signal_handling::create_combined_shutdown_signal;
/// use tracing::info;
///
/// # async fn example() {
/// let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
/// let shutdown_signal = create_combined_shutdown_signal(async {
///     shutdown_rx.await.ok();
/// });
///
/// tokio::select! {
///     _ = async { /* service_work */ } => {},
///     _ = shutdown_signal => {
///         info!("Received shutdown signal");
///     }
/// }
/// # }
/// ```
pub async fn create_combined_shutdown_signal<F>(manual_shutdown: F)
where
    F: std::future::Future<Output = ()>,
{
    tokio::select! {
        _ = handle_system_signals() => {
            info!("Received system shutdown signal");
        }
        _ = manual_shutdown => {
            info!("Received manual shutdown signal");
        }
    }
}

/// Create a shutdown signal with service-specific logging
///
/// This provides service-specific logging messages for better debugging.
///
/// # Arguments
/// * `service_name` - Name of the service for logging context
///
/// # Example
/// ```rust,no_run
/// use voice_cli::utils::signal_handling::create_service_shutdown_signal;
///
/// # async fn example() {
/// let shutdown_signal = create_service_shutdown_signal("http-server");
/// tokio::select! {
///     _ = async { /* service_work */ } => {},
///     _ = shutdown_signal => {}
/// }
/// # }
/// ```
pub async fn create_service_shutdown_signal(service_name: &str) {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failed to listen for Ctrl+C in {}: {}", service_name, e);
        } else {
            info!("{} received Ctrl+C signal", service_name);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
                info!("{} received SIGTERM signal", service_name);
            }
            Err(e) => {
                error!("Failed to listen for SIGTERM in {}: {}", service_name, e);
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => debug!("{} handled Ctrl+C signal", service_name),
        _ = terminate => debug!("{} handled SIGTERM signal", service_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn test_signal_handling_timeout() {
        // Test that the signal handlers don't hang indefinitely
        let result = timeout(Duration::from_millis(100), create_shutdown_signal()).await;

        // Should timeout since no signals are sent
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_combined_shutdown_manual() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        // Start the combined signal handler
        let signal_future = create_combined_shutdown_signal(async move {
            rx.await.ok();
        });

        // Send manual shutdown signal
        let _ = tx.send(());

        // Should complete quickly due to manual signal
        let result = timeout(Duration::from_millis(100), signal_future).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_service_shutdown_signal_timeout() {
        // Test service-specific signal handling
        let result = timeout(
            Duration::from_millis(100),
            create_service_shutdown_signal("test-service"),
        )
        .await;

        // Should timeout since no signals are sent
        assert!(result.is_err());
    }
}
