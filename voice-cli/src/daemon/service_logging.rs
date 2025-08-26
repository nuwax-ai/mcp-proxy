//! Logging integration for background services
//! 
//! This module provides unified logging configuration for background services,
//! integrating with the existing voice-cli logging infrastructure while supporting
//! both foreground and background execution modes.

use crate::models::Config;
use crate::utils::init_logging;
use std::path::PathBuf;
use tracing::{info, warn, error};

/// Initialize logging for background services using existing voice-cli infrastructure
/// 
/// This function integrates with the existing `init_logging` function while providing
/// appropriate behavior for both foreground and background service modes.
/// 
/// # Arguments
/// * `config` - The voice-cli configuration containing logging settings
/// * `service_name` - Name of the service for logging context
/// * `foreground_mode` - true for 'run' commands, false for 'start'/'restart'
/// 
/// # Behavior
/// - **Foreground mode**: Uses existing init_logging with console + file output
/// - **Background mode**: Uses existing init_logging (which already handles file-only properly)
/// - **Log directory**: Uses configured `log_dir` from config, not hardcoded
/// - **Log rotation**: Handled by existing tracing_appender infrastructure
pub fn init_service_logging(
    config: &Config, 
    service_name: &str, 
    foreground_mode: bool
) -> Result<(), Box<dyn std::error::Error>> {
    // The existing init_logging function already handles both console and file logging
    // appropriately, so we just need to call it and provide user feedback
    
    if foreground_mode {
        // Foreground mode: init_logging will output to both console and file
        init_logging(config)?;
        
        // Inform user where logs are also being written
        let log_dir = get_logs_directory(config);
        let log_file = log_dir.join("voice-cli.log");
        
        info!("Starting {} service in foreground mode", service_name);
        info!("Logs are also written to: {}", log_file.display());
        
        // Print to stdout for immediate user feedback (before logging is fully initialized)
        println!("📋 {} service starting in foreground mode", service_name);
        println!("📋 Logs are written to: {}", log_file.display());
        println!("📋 Press Ctrl+C to stop the service");
    } else {
        // Background mode: init_logging already handles file-only logging appropriately
        // The existing implementation correctly suppresses console output in daemon mode
        init_logging(config)?;
        
        let log_dir = get_logs_directory(config);
        let log_file = log_dir.join("voice-cli.log");
        
        info!("Starting {} service in background mode", service_name);
        info!("Service logs directory: {}", log_dir.display());
        
        // For background mode, we still want some immediate feedback before logging init
        eprintln!("📋 {} service starting in background mode", service_name);
        eprintln!("📋 Logs directory: {}", log_dir.display());
        eprintln!("📋 Log file: {}", log_file.display());
    }
    
    Ok(())
}

/// Get the configured logs directory from config
/// 
/// This uses the existing `log_dir_path()` method from the Config implementation,
/// ensuring we respect the user's configured log directory.
pub fn get_logs_directory(config: &Config) -> PathBuf {
    config.log_dir_path()
}

/// Validate logging configuration for background services
/// 
/// Ensures the logging configuration is valid for the service context.
pub fn validate_logging_config(config: &Config) -> Result<(), LoggingError> {
    // Check if log directory is writable
    let log_dir = get_logs_directory(config);
    
    // Create directory if it doesn't exist
    if !log_dir.exists() {
        std::fs::create_dir_all(&log_dir)
            .map_err(|e| LoggingError::DirectoryCreation {
                path: log_dir.clone(),
                error: e.to_string(),
            })?;
    }
    
    // Test write permissions
    let test_file = log_dir.join(".write_test");
    match std::fs::write(&test_file, "test") {
        Ok(_) => {
            // Clean up test file
            let _ = std::fs::remove_file(&test_file);
        }
        Err(e) => {
            return Err(LoggingError::PermissionDenied {
                path: log_dir,
                error: e.to_string(),
            });
        }
    }
    
    // Validate log level
    if !is_valid_log_level(&config.logging.level) {
        return Err(LoggingError::InvalidLogLevel {
            level: config.logging.level.clone(),
        });
    }
    
    Ok(())
}

/// Check if a log level string is valid
fn is_valid_log_level(level: &str) -> bool {
    matches!(level.to_lowercase().as_str(), "trace" | "debug" | "info" | "warn" | "warning" | "error")
}

/// Setup log rotation and cleanup for background services
/// 
/// This function sets up additional log management for long-running background services.
pub fn setup_log_rotation(config: &Config, service_name: &str) -> Result<(), LoggingError> {
    let log_dir = get_logs_directory(config);
    
    // The existing logging system already handles rotation via tracing_appender::rolling::RollingFileAppender
    // We just need to ensure the service-specific log files are in the right place
    
    info!("Log rotation configured for {} service", service_name);
    info!("Log directory: {}", log_dir.display());
    info!("Daily rotation enabled via tracing_appender");
    info!("Max files: {}", config.logging.max_files);
    
    Ok(())
}

/// Get environment variable overrides for logging configuration
/// 
/// This supports the existing environment variable override system.
pub fn get_logging_env_overrides() -> LoggingEnvOverrides {
    LoggingEnvOverrides {
        log_level: std::env::var("VOICE_CLI_LOG_LEVEL").ok(),
        log_dir: std::env::var("VOICE_CLI_LOG_DIR").ok(),
        max_files: std::env::var("VOICE_CLI_LOG_MAX_FILES")
            .ok()
            .and_then(|s| s.parse().ok()),
    }
}

/// Apply environment variable overrides to logging configuration
/// 
/// This modifies the config in-place with any environment variable overrides.
pub fn apply_logging_env_overrides(config: &mut Config) {
    let overrides = get_logging_env_overrides();
    
    if let Some(level) = overrides.log_level {
        info!("Overriding log level from environment: {}", level);
        config.logging.level = level;
    }
    
    if let Some(dir) = overrides.log_dir {
        info!("Overriding log directory from environment: {}", dir);
        config.logging.log_dir = dir;
    }
    
    if let Some(max_files) = overrides.max_files {
        info!("Overriding max log files from environment: {}", max_files);
        config.logging.max_files = max_files;
    }
}

/// Create service-specific log context
/// 
/// This provides structured logging context for different services.
pub fn create_service_log_context(service_name: &str, foreground_mode: bool) -> ServiceLogContext {
    ServiceLogContext {
        service_name: service_name.to_string(),
        mode: if foreground_mode { "foreground" } else { "background" },
        start_time: std::time::Instant::now(),
    }
}

/// Environment variable overrides for logging
#[derive(Debug, Clone)]
pub struct LoggingEnvOverrides {
    pub log_level: Option<String>,
    pub log_dir: Option<String>,
    pub max_files: Option<u32>,
}

/// Service-specific logging context
#[derive(Debug, Clone)]
pub struct ServiceLogContext {
    pub service_name: String,
    pub mode: &'static str,
    pub start_time: std::time::Instant,
}

impl ServiceLogContext {
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }
}

/// Logging-related errors
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("Failed to create log directory {path}: {error}")]
    DirectoryCreation {
        path: PathBuf,
        error: String,
    },
    
    #[error("Permission denied for log directory {path}: {error}")]
    PermissionDenied {
        path: PathBuf,
        error: String,
    },
    
    #[error("Invalid log level '{level}'. Valid levels: trace, debug, info, warn, error")]
    InvalidLogLevel {
        level: String,
    },
    
    #[error("Logging initialization failed: {0}")]
    InitializationFailed(String),
}

/// Cleanup logs when service stops
/// 
/// This performs any necessary log cleanup when a background service stops.
pub fn cleanup_service_logs(config: &Config, service_name: &str) {
    // The existing cleanup_old_logs function in utils handles general log cleanup
    // We just need to log that the service is stopping
    
    info!("Cleaning up logs for {} service", service_name);
    
    // Call existing cleanup function
    if let Err(e) = crate::utils::cleanup_old_logs(config) {
        warn!("Failed to cleanup old logs: {}", e);
    }
}

/// Log service startup information
pub fn log_service_startup(
    config: &Config,
    service_name: &str,
    foreground_mode: bool,
    additional_info: Option<&str>,
) {
    let mode = if foreground_mode { "foreground" } else { "background" };
    let log_dir = get_logs_directory(config);
    
    info!("=== {} Service Startup ===", service_name.to_uppercase());
    info!("Service: {}", service_name);
    info!("Mode: {}", mode);
    info!("Log Level: {}", config.logging.level);
    info!("Log Directory: {}", log_dir.display());
    info!("Max Log Files: {}", config.logging.max_files);
    
    if let Some(info) = additional_info {
        info!("Additional Info: {}", info);
    }
    
    // Log environment overrides if any
    let overrides = get_logging_env_overrides();
    if overrides.log_level.is_some() || overrides.log_dir.is_some() || overrides.max_files.is_some() {
        info!("Environment Overrides Active:");
        if let Some(level) = &overrides.log_level {
            info!("  VOICE_CLI_LOG_LEVEL: {}", level);
        }
        if let Some(dir) = &overrides.log_dir {
            info!("  VOICE_CLI_LOG_DIR: {}", dir);
        }
        if let Some(max_files) = overrides.max_files {
            info!("  VOICE_CLI_LOG_MAX_FILES: {}", max_files);
        }
    }
    
    info!("========================================");
}

/// Log service shutdown information  
pub fn log_service_shutdown(service_name: &str, uptime: Option<std::time::Duration>) {
    info!("=== {} Service Shutdown ===", service_name.to_uppercase());
    info!("Service: {}", service_name);
    
    if let Some(uptime) = uptime {
        info!("Uptime: {:?}", uptime);
    }
    
    info!("========================================");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Config, LoggingConfig};
    use tempfile::TempDir;

    fn create_test_config(log_dir: &str) -> Config {
        let mut config = Config::default();
        config.logging = LoggingConfig {
            level: "info".to_string(),
            log_dir: log_dir.to_string(),
            max_file_size: "10MB".to_string(),
            max_files: 5,
        };
        config
    }

    #[test]
    fn test_get_logs_directory() {
        let config = create_test_config("./test_logs");
        let log_dir = get_logs_directory(&config);
        assert_eq!(log_dir, PathBuf::from("./test_logs"));
    }

    #[test]
    fn test_validate_logging_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = create_test_config(temp_dir.path().to_str().unwrap());
        
        let result = validate_logging_config(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_log_level() {
        let config = Config {
            logging: LoggingConfig {
                level: "invalid".to_string(),
                log_dir: "./logs".to_string(),
                max_file_size: "10MB".to_string(),
                max_files: 5,
            },
            ..Default::default()
        };
        
        let result = validate_logging_config(&config);
        assert!(matches!(result, Err(LoggingError::InvalidLogLevel { .. })));
    }

    #[test]
    fn test_is_valid_log_level() {
        assert!(is_valid_log_level("info"));
        assert!(is_valid_log_level("INFO"));
        assert!(is_valid_log_level("debug"));
        assert!(is_valid_log_level("error"));
        assert!(is_valid_log_level("warn"));
        assert!(is_valid_log_level("warning"));
        assert!(is_valid_log_level("trace"));
        assert!(!is_valid_log_level("invalid"));
    }

    #[test]
    fn test_service_log_context() {
        let context = create_service_log_context("test-service", true);
        assert_eq!(context.service_name, "test-service");
        assert_eq!(context.mode, "foreground");
        
        // Test uptime
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(context.uptime().as_millis() >= 10);
    }

    #[test]
    fn test_logging_env_overrides() {
        // Set environment variables
        unsafe {
            std::env::set_var("VOICE_CLI_LOG_LEVEL", "debug");
            std::env::set_var("VOICE_CLI_LOG_DIR", "/custom/logs");
            std::env::set_var("VOICE_CLI_LOG_MAX_FILES", "10");
        }
        
        let overrides = get_logging_env_overrides();
        assert_eq!(overrides.log_level, Some("debug".to_string()));
        assert_eq!(overrides.log_dir, Some("/custom/logs".to_string()));
        assert_eq!(overrides.max_files, Some(10));
        
        // Clean up
        unsafe {
            std::env::remove_var("VOICE_CLI_LOG_LEVEL");
            std::env::remove_var("VOICE_CLI_LOG_DIR");
            std::env::remove_var("VOICE_CLI_LOG_MAX_FILES");
        }
    }

    #[test]
    fn test_apply_logging_env_overrides() {
        unsafe {
            std::env::set_var("VOICE_CLI_LOG_LEVEL", "debug");
        }
        
        let mut config = create_test_config("./logs");
        assert_eq!(config.logging.level, "info");
        
        apply_logging_env_overrides(&mut config);
        assert_eq!(config.logging.level, "debug");
        
        unsafe {
            std::env::remove_var("VOICE_CLI_LOG_LEVEL");
        }
    }
}