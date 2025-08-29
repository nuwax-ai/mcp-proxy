#[cfg(test)]
mod config_validation_tests {
    use crate::models::{
        AudioProcessingConfig, Config, DaemonConfig,
        LoggingConfig, ServerConfig, TaskManagementConfig, WhisperConfig, WorkersConfig,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper function to create a valid base configuration
    fn create_valid_config() -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                max_file_size: 25 * 1024 * 1024, // 25MB
                cors_enabled: true,
            },
            whisper: WhisperConfig {
                default_model: "base".to_string(),
                models_dir: "./models".to_string(),
                auto_download: true,
                supported_models: vec!["base".to_string()],
                audio_processing: AudioProcessingConfig::default(),
                workers: WorkersConfig {
                    transcription_workers: 2,
                    channel_buffer_size: 100,
                    worker_timeout: 3600,
                },
            },
            logging: LoggingConfig {
                level: "info".to_string(),
                log_dir: "./logs".to_string(),
                max_file_size: "10MB".to_string(),
                max_files: 10,
            },
            daemon: DaemonConfig {
                pid_file: "./voice_cli.pid".to_string(),
                log_file: "./logs/daemon.log".to_string(),
                work_dir: "./work".to_string(),
            },
            task_management: TaskManagementConfig::default(),
        }
    }

    #[test]
    fn test_valid_config_validation() {
        let config = create_valid_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_empty_host_validation() {
        let mut config = create_valid_config();
        config.server.host = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Server host cannot be empty"));
    }

    #[test]
    fn test_invalid_port_validation() {
        let mut config = create_valid_config();
        config.server.port = 0;

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Server port must be between 1 and 65535"));
    }

    #[test]
    fn test_port_too_high_validation() {
        // Note: We can't test with 70000 as it doesn't fit in u16
        // This test verifies the validation logic exists, but the type system
        // already prevents invalid port values at compile time
        let mut config = create_valid_config();
        config.server.port = 65535; // Maximum valid port

        let result = config.validate();
        assert!(result.is_ok()); // Should be valid
    }

    #[test]
    fn test_zero_max_file_size_validation() {
        let mut config = create_valid_config();
        config.server.max_file_size = 0;

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Max file size must be greater than 0"));
    }

    #[test]
    fn test_empty_default_model_validation() {
        let mut config = create_valid_config();
        config.whisper.default_model = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Default model cannot be empty"));
    }

    #[test]
    fn test_empty_models_dir_validation() {
        let mut config = create_valid_config();
        config.whisper.models_dir = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Models directory cannot be empty"));
    }

    #[test]
    fn test_zero_transcription_workers_validation() {
        let mut config = create_valid_config();
        config.whisper.workers.transcription_workers = 0;

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Transcription workers must be greater than 0"));
    }

    #[test]
    fn test_invalid_log_level_validation() {
        let mut config = create_valid_config();
        config.logging.level = "invalid".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Invalid log level"));
    }

    #[test]
    fn test_valid_log_levels() {
        let valid_levels = ["trace", "debug", "info", "warn", "error"];

        for level in &valid_levels {
            let mut config = create_valid_config();
            config.logging.level = level.to_string();

            let result = config.validate();
            assert!(result.is_ok(), "Log level '{}' should be valid", level);
        }
    }

    #[test]
    fn test_case_insensitive_log_levels() {
        let levels = ["TRACE", "Debug", "INFO", "Warn", "ERROR"];

        for level in &levels {
            let mut config = create_valid_config();
            config.logging.level = level.to_string();

            let result = config.validate();
            assert!(
                result.is_ok(),
                "Log level '{}' should be valid (case insensitive)",
                level
            );
        }
    }

    #[test]
    fn test_empty_log_dir_validation() {
        let mut config = create_valid_config();
        config.logging.log_dir = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Log directory cannot be empty"));
    }

    #[test]
    fn test_zero_max_files_validation() {
        let mut config = create_valid_config();
        config.logging.max_files = 0;

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Max log files must be greater than 0"));
    }

    #[test]
    fn test_empty_work_dir_validation() {
        let mut config = create_valid_config();
        config.daemon.work_dir = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("Work directory cannot be empty"));
    }

    #[test]
    fn test_empty_pid_file_validation() {
        let mut config = create_valid_config();
        config.daemon.pid_file = "".to_string();

        let result = config.validate();
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert!(error.to_string().contains("PID file path cannot be empty"));
    }

    #[test]
    fn test_multiple_validation_errors() {
        let mut config = create_valid_config();
        config.server.host = "".to_string();
        config.server.port = 0;
        config.whisper.default_model = "".to_string();
        config.logging.level = "invalid".to_string();

        let result = config.validate();
        assert!(result.is_err());

        // Should report the first error encountered
        let error = result.unwrap_err();
        assert!(error.to_string().contains("Server host cannot be empty"));
    }

    #[test]
    fn test_config_file_loading_and_validation() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.yml");

        // Write invalid config
        let invalid_config_yaml = r#"
server:
  host: ""
  port: 0
whisper:
  default_model: ""
"#;
        std::fs::write(&config_path, invalid_config_yaml).unwrap();

        // Loading should fail due to validation
        let result = Config::load_or_create(&config_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_path_helpers() {
        let config = create_valid_config();

        // Test path helper methods
        let models_path = config.models_dir_path();
        assert_eq!(models_path, PathBuf::from("./models"));

        let log_path = config.log_dir_path();
        assert_eq!(log_path, PathBuf::from("./logs"));
    }

    #[test]
    fn test_edge_case_port_values() {
        let mut config = create_valid_config();

        // Test minimum valid port
        config.server.port = 1;
        assert!(config.validate().is_ok());

        // Test maximum valid port
        config.server.port = 65535;
        assert!(config.validate().is_ok());

        // Note: Can't test 65536 as it doesn't fit in u16
        // The type system prevents invalid port values at compile time
    }

    #[test]
    fn test_transcription_workers_boundary() {
        let mut config = create_valid_config();

        // Test minimum valid workers
        config.whisper.workers.transcription_workers = 1;
        assert!(config.validate().is_ok());

        // Test high number of workers
        config.whisper.workers.transcription_workers = 100;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_max_files_boundary() {
        let mut config = create_valid_config();

        // Test minimum valid max files
        config.logging.max_files = 1;
        assert!(config.validate().is_ok());

        // Test high number of files
        config.logging.max_files = 1000;
        assert!(config.validate().is_ok());
    }
}