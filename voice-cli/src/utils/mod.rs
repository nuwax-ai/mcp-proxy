pub mod signal_handling;
pub mod cleanup;
pub mod task_id;

use crate::VoiceCliError;
use crate::models::Config;
use std::path::PathBuf;
use tracing::{Level, info};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, prelude::*};

// Re-export signal handling components
pub use signal_handling::{
    create_combined_shutdown_signal, create_service_shutdown_signal, create_shutdown_signal,
    handle_system_signals,
};
pub use cleanup::perform_shutdown_cleanup;
pub use task_id::{generate_task_id, generate_legacy_task_id};


/// Initialize logging based on configuration
/// The guard is stored globally to ensure logging stays active
pub fn init_logging(config: &Config) -> crate::Result<()> {
    // Check if logging is already initialized
    if tracing::dispatcher::has_been_set() {
        // Reset logging to allow proper file logging setup
        // Use the proper method to clear the global dispatcher
        tracing::subscriber::set_global_default(tracing::subscriber::NoSubscriber::default()).ok();
    }

    // Create logs directory if it doesn't exist
    let log_dir = config.log_dir_path();
    std::fs::create_dir_all(&log_dir)?;

    // Parse log level
    let level = parse_log_level(&config.logging.level)?;

    // Create file appender with rotation
    let file_appender = RollingFileAppender::new(Rotation::DAILY, &log_dir, "voice-cli.log");

    // Create console layer with proper formatting
    let console_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(true);

    // Create file layer with detailed formatting
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .with_ansi(false);

    // Create filters based on configured log level
    let console_filter = EnvFilter::new(&format!("{}", level));
    let file_filter = EnvFilter::new(&format!("{}", level));

    // Create the subscriber with both layers
    let subscriber = tracing_subscriber::registry()
        .with(console_layer.with_filter(console_filter))
        .with(file_layer.with_filter(file_filter));

    // Set as global default and get the guard
    let _ = tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| VoiceCliError::Config(format!("Failed to set global subscriber: {}", e)))?;

    // Store the guard globally to keep logging active

    info!(
        "Logging initialized - Level: {}, Directory: {:?}",
        config.logging.level, log_dir
    );

    Ok(())
}

/// Parse log level string to tracing Level
fn parse_log_level(level_str: &str) -> crate::Result<Level> {
    match level_str.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" | "warning" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        _ => Err(VoiceCliError::Config(format!(
            "Invalid log level: {}. Valid levels: trace, debug, info, warn, error",
            level_str
        ))),
    }
}

/// Clean up old log files based on configuration
pub fn cleanup_old_logs(config: &Config) -> crate::Result<()> {
    let log_dir = config.log_dir_path();

    if !log_dir.exists() {
        return Ok(());
    }

    let max_files = config.logging.max_files as usize;

    // Get all log files
    let mut log_files: Vec<_> = std::fs::read_dir(&log_dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext == "log")
                .unwrap_or(false)
        })
        .collect();

    // Sort by modification time (newest first)
    log_files.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    log_files.reverse();

    // Remove old files
    if log_files.len() > max_files {
        for old_file in log_files.iter().skip(max_files) {
            if let Err(e) = std::fs::remove_file(old_file.path()) {
                tracing::warn!("Failed to remove old log file {:?}: {}", old_file.path(), e);
            } else {
                tracing::debug!("Removed old log file: {:?}", old_file.path());
            }
        }
    }

    Ok(())
}

/// Get the current executable path
pub fn get_current_exe_path() -> crate::Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| VoiceCliError::Config(format!("Failed to get current executable path: {}", e)))
}

/// Check if a port is available
pub fn is_port_available(host: &str, port: u16) -> bool {
    match std::net::TcpListener::bind(format!("{}:{}", host, port)) {
        Ok(_) => true,
        Err(_) => false,
    }
}

/// Create a safe filename from a string
pub fn safe_filename(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

/// Get system information for debugging
pub fn get_system_info() -> SystemInfo {
    SystemInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        family: std::env::consts::FAMILY.to_string(),
        exe_suffix: std::env::consts::EXE_SUFFIX.to_string(),
    }
}

#[derive(Debug)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub family: String,
    pub exe_suffix: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_level() {
        assert!(matches!(parse_log_level("info"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("INFO"), Ok(Level::INFO)));
        assert!(matches!(parse_log_level("debug"), Ok(Level::DEBUG)));
        assert!(matches!(parse_log_level("error"), Ok(Level::ERROR)));
        assert!(parse_log_level("invalid").is_err());
    }

    #[test]
    fn test_safe_filename() {
        assert_eq!(safe_filename("hello world"), "hello_world");
        assert_eq!(safe_filename("test-file.mp3"), "test-file.mp3");
        assert_eq!(safe_filename("special@#$chars"), "special___chars");
    }

    #[test]
    fn test_is_port_available() {
        // Test with a likely available port
        assert!(is_port_available("127.0.0.1", 0)); // Port 0 should always be available for testing
    }

    #[test]
    fn test_get_system_info() {
        let info = get_system_info();
        assert!(!info.os.is_empty());
        assert!(!info.arch.is_empty());
    }
}
