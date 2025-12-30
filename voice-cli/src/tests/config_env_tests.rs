#[cfg(test)]
mod config_env_tests {
    use crate::models::Config;
    use std::collections::HashMap;
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use tempfile::TempDir;

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
            "VOICE_CLI_LOG_LEVEL",
            "VOICE_CLI_LOG_DIR",
            "VOICE_CLI_LOG_MAX_FILES",
            "VOICE_CLI_DEFAULT_MODEL",
            "VOICE_CLI_MODELS_DIR",
            "VOICE_CLI_AUTO_DOWNLOAD",
            "VOICE_CLI_TRANSCRIPTION_WORKERS",
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
                Some(value) => unsafe {
                    env::set_var(key, value);
                },
                None => unsafe {
                    env::remove_var(key);
                },
            }
        }
    }

    #[test]
    fn test_http_port_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for HTTP port
        safe_set_env_var("VOICE_CLI_PORT", "9090");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify HTTP port was overridden
        assert_eq!(config.server.port, 9090);

        // Clean up
        safe_remove_env_var("VOICE_CLI_PORT");
    }

    #[test]
    fn test_invalid_port_environment_variable() {
        clear_voice_cli_env_vars();

        // Set invalid environment variable
        safe_set_env_var("VOICE_CLI_PORT", "invalid_port");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("Invalid VOICE_CLI_PORT value 'invalid_port'"));

        // Clean up
        safe_remove_env_var("VOICE_CLI_PORT");
    }

    #[test]
    fn test_log_level_environment_override() {
        clear_voice_cli_env_vars();

        // Set environment variable for log level
        safe_set_env_var("VOICE_CLI_LOG_LEVEL", "DEBUG");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify log level was overridden and normalized to lowercase
        assert_eq!(config.logging.level, "debug");

        // Clean up
        safe_remove_env_var("VOICE_CLI_LOG_LEVEL");
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

        // Set multiple environment variables
        // 注意：使用有效的模型名称（large-v3 而不是 large）
        safe_set_env_var("VOICE_CLI_PORT", "8081");
        safe_set_env_var("VOICE_CLI_LOG_LEVEL", "warn");
        safe_set_env_var("VOICE_CLI_DEFAULT_MODEL", "large-v3");

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let config = Config::load_with_env_overrides(&config_path).unwrap();

        // Verify all overrides were applied
        assert_eq!(config.server.port, 8081);
        assert_eq!(config.logging.level, "warn");
        assert_eq!(config.whisper.default_model, "large-v3");

        // Verify validation passes
        assert!(config.validate().is_ok());

        // Clean up
        safe_remove_env_var("VOICE_CLI_PORT");
        safe_remove_env_var("VOICE_CLI_LOG_LEVEL");
        safe_remove_env_var("VOICE_CLI_DEFAULT_MODEL");
    }

    #[test]
    fn test_empty_environment_variable_validation() {
        clear_voice_cli_env_vars();

        // Set empty environment variable
        safe_set_env_var("VOICE_CLI_HOST", "   "); // Use spaces instead of empty string

        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        let result = Config::load_with_env_overrides(&config_path);

        // Should fail with proper error message
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("VOICE_CLI_HOST environment variable cannot be empty"));

        // Clean up
        safe_remove_env_var("VOICE_CLI_HOST");
    }
}
