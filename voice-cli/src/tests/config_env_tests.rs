#[cfg(test)]
mod config_env_tests {
    use crate::models::Config;
    use std::env;
    use tempfile::TempDir;
    use std::sync::{Mutex, OnceLock};
    use std::collections::HashMap;

    // Safe environment variable testing using static state
    static TEST_ENV_LOCK: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

    fn get_test_env_lock() -> &'static Mutex<HashMap<String, Option<String>>> {
        TEST_ENV_LOCK.get_or_init(|| Mutex::new(HashMap::new()))
    }

    // Safe helper to set environment variable for testing
    fn safe_set_env_var(key: &str, value: &str) {
        let lock = get_test_env_lock();
        let mut env_state = lock.lock().unwrap();
        
        // Store the original value if this is the first time setting this var
        if !env_state.contains_key(key) {
            env_state.insert(key.to_string(), env::var(key).ok());
        }
        
        // This is still technically unsafe, but we'll wrap it in unsafe block
        // and document that tests should run serially to avoid race conditions
        unsafe {
            env::set_var(key, value);
        }
    }

    // Safe helper to remove environment variable for testing
    fn safe_remove_env_var(key: &str) {
        let lock = get_test_env_lock();
        let mut env_state = lock.lock().unwrap();
        
        // Store the original value if this is the first time touching this var
        if !env_state.contains_key(key) {
            env_state.insert(key.to_string(), env::var(key).ok());
        }
        
        unsafe {
            env::remove_var(key);
        }
    }

    // Helper function to clear all voice-cli environment variables safely
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
            safe_remove_env_var(var);
        }

        // Add a small delay to ensure environment changes propagate
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Cleanup function to restore original environment state
    fn restore_original_env_vars() {
        let lock = get_test_env_lock();
        let env_state = lock.lock().unwrap();
        
        for (key, original_value) in env_state.iter() {
            match original_value {
                Some(value) => unsafe { env::set_var(key, value); },
                None => unsafe { env::remove_var(key); },
            }
        }
    }

    #[test]
    fn test_http_port_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for HTTP port
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "9090");
        // Ensure gRPC port is different to avoid conflicts
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "50051");

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
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
    }

    #[test]
    fn test_grpc_port_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for gRPC port
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "50052");
        // Ensure HTTP port is different to avoid conflicts
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "8080");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify gRPC port was overridden
        assert_eq!(config.cluster.grpc_port, 50052);
        assert_eq!(config.cluster.http_port, 8080); // Should be different

        // Verify cluster is disabled by default, so no port conflict validation
        assert!(!config.cluster.enabled);

        // Clean up
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
    }

    #[test]
    fn test_invalid_port_environment_variable() {
        clear_voice_cli_env_vars();

        // Set invalid environment variable
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "invalid_port");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_HTTP_PORT value 'invalid_port'"));

        // Clean up
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
    }

    #[test]
    fn test_cluster_enabled_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variables to avoid port conflicts and ensure valid cluster config
        safe_set_env_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "8080");
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "50051");
        // Set election timeout to satisfy 5x heartbeat interval requirement
        safe_set_env_var("VOICE_CLI_HEARTBEAT_INTERVAL", "3");
        safe_set_env_var("VOICE_CLI_ELECTION_TIMEOUT", "15"); // 5 * 3

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
        safe_remove_env_var("VOICE_CLI_CLUSTER_ENABLED");
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
        safe_remove_env_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        safe_remove_env_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_log_level_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for log level
        safe_set_env_var("VOICE_CLI_LOG_LEVEL", "DEBUG");
        // Ensure different ports to avoid conflicts when validating
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "8080");
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "50051");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify log level was overridden and normalized to lowercase
        assert_eq!(config.logging.level, "debug");

        // Verify cluster is disabled by default
        assert!(!config.cluster.enabled);

        // Clean up
        safe_remove_env_var("VOICE_CLI_LOG_LEVEL");
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
    }

    #[test]
    fn test_invalid_log_level_environment_variable() {
        clear_voice_cli_env_vars();

        // Set invalid environment variable
        safe_set_env_var("VOICE_CLI_LOG_LEVEL", "invalid_level");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_LOG_LEVEL"));
        assert!(error_msg.contains("invalid_level"));

        // Clean up
        safe_remove_env_var("VOICE_CLI_LOG_LEVEL");
    }

    #[test]
    fn test_comprehensive_validation() {
        clear_voice_cli_env_vars();

        // Set multiple environment variables with non-conflicting ports and valid cluster timing
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "8081");
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "50053");
        safe_set_env_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        safe_set_env_var("VOICE_CLI_NODE_ID", "test-node-123");
        safe_set_env_var("VOICE_CLI_LOG_LEVEL", "warn");
        // Ensure valid cluster timing configuration
        safe_set_env_var("VOICE_CLI_HEARTBEAT_INTERVAL", "4");
        safe_set_env_var("VOICE_CLI_ELECTION_TIMEOUT", "20"); // 5 * 4

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
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
        safe_remove_env_var("VOICE_CLI_CLUSTER_ENABLED");
        safe_remove_env_var("VOICE_CLI_NODE_ID");
        safe_remove_env_var("VOICE_CLI_LOG_LEVEL");
        safe_remove_env_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        safe_remove_env_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_port_conflict_validation() {
        clear_voice_cli_env_vars();

        // Set conflicting ports and valid cluster timing to test port conflict specifically
        safe_set_env_var("VOICE_CLI_HTTP_PORT", "8080");
        safe_set_env_var("VOICE_CLI_GRPC_PORT", "8080");
        safe_set_env_var("VOICE_CLI_CLUSTER_ENABLED", "true");
        // Set valid cluster timing to ensure port conflict is the only issue
        safe_set_env_var("VOICE_CLI_HEARTBEAT_INTERVAL", "3");
        safe_set_env_var("VOICE_CLI_ELECTION_TIMEOUT", "15"); // 5 * 3

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
        safe_remove_env_var("VOICE_CLI_HTTP_PORT");
        safe_remove_env_var("VOICE_CLI_GRPC_PORT");
        safe_remove_env_var("VOICE_CLI_CLUSTER_ENABLED");
        safe_remove_env_var("VOICE_CLI_HEARTBEAT_INTERVAL");
        safe_remove_env_var("VOICE_CLI_ELECTION_TIMEOUT");
    }

    #[test]
    fn test_empty_environment_variable_validation() {
        clear_voice_cli_env_vars();

        // Set empty environment variable
        safe_set_env_var("VOICE_CLI_NODE_ID", "   "); // Use spaces instead of empty string

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("VOICE_CLI_NODE_ID environment variable cannot be empty"));

        // Clean up
        safe_remove_env_var("VOICE_CLI_NODE_ID");
    }
}

