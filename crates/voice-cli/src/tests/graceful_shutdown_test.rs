#[cfg(test)]
mod graceful_shutdown_tests {
    use crate::models::Config;
    use crate::server;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tracing::info;

    #[tokio::test]
    async fn test_graceful_shutdown_mechanism() {
        // Test that the broadcast channel shutdown mechanism works
        let (shutdown_tx, _) = broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();

        // Test sending and receiving shutdown signals
        let send_handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            info!("Sending shutdown signal...");
            shutdown_tx
                .send(())
                .expect("Failed to send shutdown signal");
        });

        let receive_handle = tokio::spawn(async move {
            info!("Waiting for shutdown signal...");
            let result = shutdown_rx.recv().await;
            assert!(result.is_ok(), "Should receive shutdown signal");
            info!("Received shutdown signal successfully");
        });

        // Wait for both tasks to complete
        let (send_result, receive_result) = tokio::join!(send_handle, receive_handle);

        assert!(
            send_result.is_ok(),
            "Send task should complete successfully"
        );
        assert!(
            receive_result.is_ok(),
            "Receive task should complete successfully"
        );
    }

    #[tokio::test]
    async fn test_app_state_shutdown() {
        // Test that AppState can be created and shut down gracefully
        let config = Config::default();
        let config_arc = Arc::new(config);

        let app_state = server::handlers::AppState::new(config_arc.clone())
            .await
            .expect("Failed to create app state");

        // Test that shutdown works
        app_state.shutdown().await;

        info!("AppState shutdown completed successfully");
    }
}
