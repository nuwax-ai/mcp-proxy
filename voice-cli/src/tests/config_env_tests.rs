#[cfg(test)]
mod config_env_tests {
    use crate::models::Config;
    use std::env;
    use tempfile::TempDir;

    // Helper function to clear all voice-cli environment variables
    fn clear_voice_cli_env_vars() {
        let env_vars = [
            "VOICE_CLI_HOST",
            "VOICE_CLI_PORT",
            "VOICE_CLI_HTTP_PORT",
            "VOICE_CLI_GRPC_PORT",
            "VOICE_CLI_NODE_ID",
            "VOICE_CLI_CLUSTER_ENABLED",
            "VOICE_CLI_BIND_ADDRESS",
            "VOICE_CLI_LEADER_CAN_PROCESS_TASKS",
            "VOICE_CLI_HEARTBEAT_INTERVAL",
            "VOICE_CLI_ELECTION_TIMEOUT",
            "VOICE_CLI_LB_ENABLED",
            "VOICE_CLI_LB_PORT",
            "VOICE_CLI_LB_BIND_ADDRESS",
            "VOICE_CLI_LB_HEALTH_CHECK_INTERVAL",
            "VOICE_CLI_LB_HEALTH_CHECK_TIMEOUT",
            "VOICE_CLI_LOG_LEVEL",
            "VOICE_CLI_LOG_DIR",
            "VOICE_CLI_LOG_MAX_FILES",
            "VOICE_CLI_DEFAULT_MODEL",
            "VOICE_CLI_MODELS_DIR",
            "VOICE_CLI_AUTO_DOWNLOAD",
            "VOICE_CLI_TRANSCRIPTION_WORKERS",
            "VOICE_CLI_METADATA_DB_PATH",
            "VOICE_CLI_WORK_DIR",
            "VOICE_CLI_PID_FILE",
            "VOICE_CLI_MAX_FILE_SIZE",
            "VOICE_CLI_CORS_ENABLED",
        ];

        for var in &env_vars {
            env::remove_var(var);
        }

        // Add a small delay to ensure environment changes propagate
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    #[test]
    fn test_http_port_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for HTTP port
        env::set_var("VOICE_CLI_HTTP_PORT", "9090");
        // Ensure gRPC port is different to avoid conflicts
        env::set_var("VOICE_CLI_GRPC_PORT", "50051");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify HTTP port was overridden
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.cluster.http_port, 9090); // Should be kept in sync
        assert_eq!(config.cluster.grpc_port, 50051); // Should be different

        // Verify cluster is disabled by default, so no port conflict validation
        assert!(!config.cluster.enabled);

        // Clean up
        env::remove_var("VOICE_CLI_HTTP_PORT");
        env::remove_var("VOICE_CLI_GRPC_PORT");
    }

    #[test]
    fn test_grpc_port_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for gRPC port
        env::set_var("VOICE_CLI_GRPC_PORT", "50052");
        // Ensure HTTP port is different to avoid conflicts
        env::set_var("VOICE_CLI_HTTP_PORT", "8080");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify gRPC port was overridden
        assert_eq!(config.cluster.grpc_port, 50052);
        assert_eq!(config.cluster.http_port, 8080); // Should be different

        // Verify cluster is disabled by default, so no port conflict validation
        assert!(!config.cluster.enabled);

        // Clean up
        env::remove_var("VOICE_CLI_GRPC_PORT");
        env::remove_var("VOICE_CLI_HTTP_PORT");
    }

    #[test]
    fn test_invalid_port_environment_variable() {
        clear_voice_cli_env_vars();

        // Set invalid environment variable
        env::set_var("VOICE_CLI_HTTP_PORT", "invalid_port");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_HTTP_PORT value 'invalid_port'"));

        // Clean up
        env::remove_var("VOICE_CLI_HTTP_PORT");
    }

    #[test]
    fn test_cluster_enabled_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variables to avoid port conflicts and ensure valid cluster config
        env::set_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        env::set_var("VOICE_CLI_HTTP_PORT", "8080");
        env::set_var("VOICE_CLI_GRPC_PORT", "50051");
        // Set election timeout to satisfy 5x heartbeat interval requirement
        env::set_var("VOICE_CLI_HEARTBEAT_INTERVAL", "3");
        env::set_var("VOICE_CLI_ELECTION_TIMEOUT", "15"); // 5 * 3

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify cluster was enabled
        assert!(config.cluster.enabled);
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.cluster.grpc_port, 50051);
        assert_eq!(config.cluster.heartbeat_interval, 3);
        assert_eq!(config.cluster.election_timeout, 15);

        // Clean up
        env::remove_var("VOICE_CLI_CLUSTER_ENABLED");
        env::remove_var("VOICE_CLI_HTTP_PORT");
        env::remove_var("VOICE_CLI_GRPC_PORT");
        env::remove_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        env::remove_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_log_level_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for log level
        env::set_var("VOICE_CLI_LOG_LEVEL", "DEBUG");
        // Ensure different ports to avoid conflicts when validating
        env::set_var("VOICE_CLI_HTTP_PORT", "8080");
        env::set_var("VOICE_CLI_GRPC_PORT", "50051");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify log level was overridden and normalized to lowercase
        assert_eq!(config.logging.level, "debug");

        // Verify cluster is disabled by default
        assert!(!config.cluster.enabled);

        // Clean up
        env::remove_var("VOICE_CLI_LOG_LEVEL");
        env::remove_var("VOICE_CLI_HTTP_PORT");
        env::remove_var("VOICE_CLI_GRPC_PORT");
    }

    #[test]
    fn test_invalid_log_level_environment_variable() {
        clear_voice_cli_env_vars();

        // Set invalid environment variable
        env::set_var("VOICE_CLI_LOG_LEVEL", "invalid_level");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_LOG_LEVEL"));
        assert!(error_msg.contains("invalid_level"));

        // Clean up
        env::remove_var("VOICE_CLI_LOG_LEVEL");
    }

    #[test]
    fn test_comprehensive_validation() {
        clear_voice_cli_env_vars();

        // Set multiple environment variables with non-conflicting ports and valid cluster timing
        env::set_var("VOICE_CLI_HTTP_PORT", "8081");
        env::set_var("VOICE_CLI_GRPC_PORT", "50053");
        env::set_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        env::set_var("VOICE_CLI_NODE_ID", "test-node-123");
        env::set_var("VOICE_CLI_LOG_LEVEL", "warn");
        // Ensure valid cluster timing configuration
        env::set_var("VOICE_CLI_HEARTBEAT_INTERVAL", "4");
        env::set_var("VOICE_CLI_ELECTION_TIMEOUT", "20"); // 5 * 4

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify all overrides were applied
        assert_eq!(config.server.port, 8081);
        assert_eq!(config.cluster.http_port, 8081);
        assert_eq!(config.cluster.grpc_port, 50053);
        assert!(config.cluster.enabled);
        assert_eq!(config.cluster.node_id, "test-node-123");
        assert_eq!(config.logging.level, "warn");
        assert_eq!(config.cluster.heartbeat_interval, 4);
        assert_eq!(config.cluster.election_timeout, 20);

        // Verify validation passes
        assert!(config.validate().is_ok());

        // Clean up
        env::remove_var("VOICE_CLI_HTTP_PORT");
        env::remove_var("VOICE_CLI_GRPC_PORT");
        env::remove_var("VOICE_CLI_CLUSTER_ENABLED");
        env::remove_var("VOICE_CLI_NODE_ID");
        env::remove_var("VOICE_CLI_LOG_LEVEL");
        env::remove_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        env::remove_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_port_conflict_validation() {
        clear_voice_cli_env_vars();

        // Set conflicting ports and valid cluster timing to test port conflict specifically
        env::set_var("VOICE_CLI_HTTP_PORT", "8080");
        env::set_var("VOICE_CLI_GRPC_PORT", "8080");
        env::set_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        // Set valid cluster timing to ensure port conflict is the only issue
        env::set_var("VOICE_CLI_HEARTBEAT_INTERVAL", "3");
        env::set_var("VOICE_CLI_ELECTION_TIMEOUT", "15"); // 5 * 3

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail due to port conflict
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("gRPC port and HTTP port cannot be the same"),
            "Expected port conflict error, got: {}",
            error_msg
        );

        // Clean up
        env::remove_var("VOICE_CLI_HTTP_PORT");
        env::remove_var("VOICE_CLI_GRPC_PORT");
        env::remove_var("VOICE_CLI_CLUSTER_ENABLED");
        env::remove_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        env::remove_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_empty_environment_variable_validation() {
        clear_voice_cli_env_vars();

        // Set empty environment variable
        env::set_var("VOICE_CLI_NODE_ID", "   "); // Use spaces instead of empty string

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("VOICE_CLI_NODE_ID environment variable cannot be empty"));

        // Clean up
        env::remove_var("VOICE_CLI_NODE_ID");
    }
}

#[cfg(test)]
mod config_hot_reload_tests {
    use crate::config::ConfigManager;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_manual_config_reload() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Create initial config manager
        let config_manager = ConfigManager::new(config_path.clone()).unwrap();
        let initial_config = config_manager.config().await;
        assert_eq!(initial_config.server.port, 8080);

        // Modify the config file
        let mut modified_config = initial_config.clone();
        modified_config.server.port = 9090;
        modified_config.save(&config_path).unwrap();

        // Wait a bit to ensure file timestamp changes
        sleep(Duration::from_millis(10)).await;

        // Manually reload
        config_manager.reload().await.unwrap();

        // Verify config was reloaded
        let reloaded_config = config_manager.config().await;
        assert_eq!(reloaded_config.server.port, 9090);
    }

    #[tokio::test]
    async fn test_config_change_notifications() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Create config manager and subscribe to changes
        let config_manager = ConfigManager::new(config_path.clone()).unwrap();
        let mut change_receiver = config_manager.subscribe_to_changes();

        // Update config programmatically
        config_manager
            .update_config(|config| {
                config.server.port = 7070;
            })
            .await
            .unwrap();

        // Verify we received a change notification
        let notification =
            tokio::time::timeout(Duration::from_secs(1), change_receiver.recv()).await;
        assert!(notification.is_ok());

        let change = notification.unwrap().unwrap();
        assert_eq!(change.old_config.server.port, 8080);
        assert_eq!(change.new_config.server.port, 7070);
    }

    #[tokio::test]
    async fn test_check_and_reload_if_changed() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Create config manager
        let config_manager = ConfigManager::new(config_path.clone()).unwrap();

        // Initially no changes
        let changed = config_manager.check_and_reload_if_changed().await.unwrap();
        assert!(!changed);

        // Modify the config file externally
        sleep(Duration::from_millis(10)).await; // Ensure timestamp difference
        let mut config = config_manager.config().await;
        config.server.port = 6060;
        config.save(&config_path).unwrap();

        // Check and reload should detect the change
        let changed = config_manager.check_and_reload_if_changed().await.unwrap();
        assert!(changed);

        // Verify config was updated
        let updated_config = config_manager.config().await;
        assert_eq!(updated_config.server.port, 6060);
    }

    #[tokio::test]
    async fn test_hot_reload_watcher() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Create config manager and start hot reload watcher
        let config_manager = ConfigManager::new(config_path.clone()).unwrap();
        let mut change_receiver = config_manager.subscribe_to_changes();

        // Start hot reload with very short interval for testing
        let _watcher_handle = config_manager.start_hot_reload(1); // Check every 1 second

        // Wait a bit then modify the config file
        sleep(Duration::from_millis(100)).await;

        let mut config = config_manager.config().await;
        config.server.port = 5050;
        config.logging.level = "debug".to_string();
        config.save(&config_path).unwrap();

        // Wait for the watcher to detect and reload the change
        let notification =
            tokio::time::timeout(Duration::from_secs(3), change_receiver.recv()).await;
        assert!(notification.is_ok());

        let change = notification.unwrap().unwrap();
        assert_eq!(change.new_config.server.port, 5050);
        assert_eq!(change.new_config.logging.level, "debug");
    }

    #[tokio::test]
    async fn test_config_validation_on_hot_reload() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Create config manager
        let config_manager = ConfigManager::new(config_path.clone()).unwrap();

        // Write invalid config to file
        let invalid_config_yaml = r#"
server:
  host: ""  # Invalid empty host
  port: 0   # Invalid port
"#;
        std::fs::write(&config_path, invalid_config_yaml).unwrap();

        // Hot reload should fail due to validation
        let result = config_manager.check_and_reload_if_changed().await;
        assert!(result.is_err());

        // Original config should remain unchanged
        let config = config_manager.config().await;
        assert_eq!(config.server.port, 8080); // Original default port
    }
}
