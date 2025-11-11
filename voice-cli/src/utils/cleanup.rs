use std::path::Path;
use tracing::{info, warn};

/// Perform global cleanup operations during shutdown
pub async fn perform_shutdown_cleanup() {
    info!("Starting shutdown cleanup operations");

    // Clean up any remaining temporary files
    cleanup_temp_directories().await;

    // Flush any remaining logs
    flush_logs().await;

    info!("Shutdown cleanup completed");
}

/// Clean up temporary directories and files
async fn cleanup_temp_directories() {
    let temp_patterns = [
        "/tmp/voice-cli-*",
        "./temp/voice-cli-*",
        "./tmp/voice-cli-*",
    ];

    for pattern in &temp_patterns {
        if let Some(parent) = Path::new(pattern).parent() {
            if parent.exists() {
                match std::fs::read_dir(parent) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if let Some(name) = path.file_name() {
                                if name.to_string_lossy().starts_with("voice-cli-") {
                                    if path.is_file() {
                                        if let Err(e) = std::fs::remove_file(&path) {
                                            warn!("Failed to cleanup temp file {:?}: {}", path, e);
                                        } else {
                                            info!("Cleaned up temp file: {:?}", path);
                                        }
                                    } else if path.is_dir() {
                                        if let Err(e) = std::fs::remove_dir_all(&path) {
                                            warn!(
                                                "Failed to cleanup temp directory {:?}: {}",
                                                path, e
                                            );
                                        } else {
                                            info!("Cleaned up temp directory: {:?}", path);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to read temp directory {:?}: {}", parent, e);
                    }
                }
            }
        }
    }
}

/// Flush any remaining logs
async fn flush_logs() {
    // Give the logging system a moment to flush any remaining logs
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Force flush tracing subscriber if possible
    // Note: This is a best-effort operation
    info!("Log flush completed");
}

/// Clean up specific temporary files
pub fn cleanup_files(files: &[std::path::PathBuf]) {
    for file in files {
        if file.exists() {
            if let Err(e) = std::fs::remove_file(file) {
                warn!("Failed to cleanup file {:?}: {}", file, e);
            } else {
                info!("Cleaned up file: {:?}", file);
            }
        }
    }
}

/// Clean up a specific directory and all its contents
pub fn cleanup_directory<P: AsRef<Path>>(dir: P) -> Result<(), std::io::Error> {
    let dir = dir.as_ref();
    if dir.exists() && dir.is_dir() {
        std::fs::remove_dir_all(dir)?;
        info!("Cleaned up directory: {:?}", dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_cleanup_files() {
        let temp_dir = TempDir::new().unwrap();
        let file1 = temp_dir.path().join("test1.txt");
        let file2 = temp_dir.path().join("test2.txt");

        // Create test files
        File::create(&file1).unwrap();
        File::create(&file2).unwrap();

        assert!(file1.exists());
        assert!(file2.exists());

        // Cleanup files
        cleanup_files(&[file1.clone(), file2.clone()]);

        assert!(!file1.exists());
        assert!(!file2.exists());
    }

    #[test]
    fn test_cleanup_directory() {
        let temp_dir = TempDir::new().unwrap();
        let test_dir = temp_dir.path().join("test_cleanup");
        std::fs::create_dir(&test_dir).unwrap();

        // Create a file in the directory
        let test_file = test_dir.join("test.txt");
        File::create(&test_file).unwrap();

        assert!(test_dir.exists());
        assert!(test_file.exists());

        // Cleanup directory
        cleanup_directory(&test_dir).unwrap();

        assert!(!test_dir.exists());
        assert!(!test_file.exists());
    }

    #[tokio::test]
    async fn test_perform_shutdown_cleanup() {
        // This test just ensures the function runs without panicking
        perform_shutdown_cleanup().await;
    }
}
