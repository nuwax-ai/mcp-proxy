use std::path::{Path, PathBuf};
use std::fs;
use bytes::Bytes;
use tracing::{info, warn, error};
use crate::VoiceCliError;
use axum::extract::multipart::Field;
use futures::{TryStreamExt};  // StreamExt 未使用，移除
use tokio::io::AsyncWriteExt;

/// Service for managing audio files on disk
#[derive(Debug, Clone)]
pub struct AudioFileManager {
    pub storage_dir: PathBuf,
}

impl AudioFileManager {
    /// Create a new AudioFileManager
    pub fn new<P: AsRef<Path>>(storage_dir: P) -> Result<Self, VoiceCliError> {
        let storage_dir = storage_dir.as_ref().to_path_buf();
        
        // Create storage directory if it doesn't exist
        if !storage_dir.exists() {
            fs::create_dir_all(&storage_dir).map_err(|e| {
                VoiceCliError::Storage(format!(
                    "Failed to create audio storage directory '{}': {}",
                    storage_dir.display(),
                    e
                ))
            })?;
        }
        
        info!("AudioFileManager initialized with storage directory: {}", storage_dir.display());
        
        Ok(Self { storage_dir })
    }
    
    /// Save audio data to disk and return the file path
    pub async fn save_audio_file(
        &self,
        task_id: &str,
        audio_data: &Bytes,
        original_filename: &str,
    ) -> Result<PathBuf, VoiceCliError> {
        // Extract file extension from original filename
        let extension = Path::new(original_filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("bin");
        
        // Create a unique filename using task_id
        let filename = format!("{}_{}.{}", task_id, uuid::Uuid::new_v4(), extension);
        let file_path = self.storage_dir.join(&filename);
        
        // Write audio data to file
        tokio::fs::write(&file_path, audio_data).await.map_err(|e| {
            VoiceCliError::Storage(format!(
                "Failed to write audio file '{}': {}",
                file_path.display(),
                e
            ))
        })?;
        
        info!(
            "Saved audio file: {} ({} bytes) -> {}",
            original_filename,
            audio_data.len(),
            file_path.display()
        );
        
        Ok(file_path)
    }
    
    /// Save audio data from multipart field stream directly to disk
    pub async fn save_audio_file_streaming(
        &self,
        task_id: &str,
        field: Field<'_>,
        temp_file_name: &str,
    ) -> Result<String, VoiceCliError> {
        // 获取原始文件名（如果有）用于日志记录
        let original_filename = field.file_name().map(|s| s.to_string()).unwrap_or_else(|| "unknown".to_string());
        info!(
            "[Task {}] 开始接收音频文件流: {}, 目标临时文件名: {}",
            task_id,
            original_filename,
            temp_file_name
        );
        
        let file_path = self.storage_dir.join(&temp_file_name);
        
        // 确保存储目录存在
        if let Some(parent) = file_path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    error!("[Task {}] 无法创建存储目录 '{}': {}", task_id, parent.display(), e);
                    VoiceCliError::Storage(format!(
                        "无法创建存储目录 '{}': {}",
                        parent.display(),
                        e
                    ))
                })?;
            }
        }
        
        // 创建文件
        let file = tokio::fs::File::create(&file_path).await.map_err(|e| {
            error!("[Task {}] 无法创建音频文件 '{}': {}", task_id, file_path.display(), e);
            VoiceCliError::Storage(format!(
                "无法创建音频文件 '{}': {}",
                file_path.display(),
                e
            ))
        })?;
        
        // 创建缓冲写入器以提高性能
        let mut writer = tokio::io::BufWriter::new(file);
        
        // 将 field 转换为 StreamReader (实现 AsyncRead trait)
        let mut reader = tokio_util::io::StreamReader::new(
            field.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        );
        
        // 使用 tokio::io::copy 进行高效的流式复制
        let total_bytes = tokio::io::copy(&mut reader, &mut writer).await.map_err(|e| {
            error!(
                "[Task {}] 流式复制音频文件数据失败 ({} -> {}): {}",
                task_id,
                original_filename,
                file_path.display(),
                e
            );
            VoiceCliError::Storage(format!(
                "流式复制音频文件数据失败 ({} -> {}): {}",
                original_filename,
                file_path.display(),
                e
            ))
        })?;
        
        // 确保所有数据都写入磁盘
        writer.flush().await.map_err(|e| {
            error!("[Task {}] 无法刷新数据到文件 '{}': {}", task_id, file_path.display(), e);
            VoiceCliError::Storage(format!(
                "无法刷新数据到文件 '{}': {}",
                file_path.display(),
                e
            ))
        })?;
        
        info!(
            "[Task {}] 成功接收并保存音频文件: {} ({} 字节) -> {}",
            task_id,
            original_filename,
            total_bytes,
            file_path.display()
        );
        
        Ok(file_path.to_string_lossy().into_owned())
    }
    
    /// Delete an audio file from disk
    pub async fn delete_audio_file<P: AsRef<Path>>(&self, file_path: P) -> Result<(), VoiceCliError> {
        let file_path = file_path.as_ref();
        
        if file_path.exists() {
            tokio::fs::remove_file(file_path).await.map_err(|e| {
                VoiceCliError::Storage(format!(
                    "Failed to delete audio file '{}': {}",
                    file_path.display(),
                    e
                ))
            })?;
            
            info!("Deleted audio file: {}", file_path.display());
        } else {
            warn!("Audio file not found for deletion: {}", file_path.display());
        }
        
        Ok(())
    }
    
    /// Delete multiple audio files
    pub async fn delete_audio_files<P: AsRef<Path>>(&self, file_paths: &[P]) -> Result<(), VoiceCliError> {
        for file_path in file_paths {
            if let Err(e) = self.delete_audio_file(file_path).await {
                // Log error but continue with other files
                error!("Failed to delete audio file: {}", e);
            }
        }
        Ok(())
    }
    
    /// Clean up old audio files based on age
    pub async fn cleanup_old_files(&self, max_age_hours: u64) -> Result<u32, VoiceCliError> {
        let mut cleaned_count = 0u32;
        let cutoff_time = std::time::SystemTime::now() - std::time::Duration::from_secs(max_age_hours * 3600);
        
        let mut entries = tokio::fs::read_dir(&self.storage_dir).await.map_err(|e| {
            VoiceCliError::Storage(format!(
                "Failed to read storage directory '{}': {}",
                self.storage_dir.display(),
                e
            ))
        })?;
        
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            VoiceCliError::Storage(format!("Failed to read directory entry: {}", e))
        })? {
            let path = entry.path();
            
            if path.is_file() {
                if let Ok(metadata) = entry.metadata().await {
                    if let Ok(modified) = metadata.modified() {
                        if modified < cutoff_time {
                            if let Err(e) = self.delete_audio_file(&path).await {
                                error!("Failed to cleanup old file '{}': {}", path.display(), e);
                            } else {
                                cleaned_count += 1;
                            }
                        }
                    }
                }
            }
        }
        
        if cleaned_count > 0 {
            info!("Cleaned up {} old audio files", cleaned_count);
        }
        
        Ok(cleaned_count)
    }
    
    /// Get the size of a file
    pub async fn get_file_size<P: AsRef<Path>>(&self, file_path: P) -> Result<u64, VoiceCliError> {
        let metadata = tokio::fs::metadata(file_path.as_ref()).await.map_err(|e| {
            VoiceCliError::Storage(format!(
                "Failed to get file metadata for '{}': {}",
                file_path.as_ref().display(),
                e
            ))
        })?;
        
        Ok(metadata.len())
    }
    
    /// Get total storage usage
    pub async fn get_storage_usage(&self) -> Result<u64, VoiceCliError> {
        let mut total_size = 0u64;
        
        let mut entries = tokio::fs::read_dir(&self.storage_dir).await.map_err(|e| {
            VoiceCliError::Storage(format!(
                "Failed to read storage directory '{}': {}",
                self.storage_dir.display(),
                e
            ))
        })?;
        
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            VoiceCliError::Storage(format!("Failed to read directory entry: {}", e))
        })? {
            if entry.path().is_file() {
                if let Ok(metadata) = entry.metadata().await {
                    total_size += metadata.len();
                }
            }
        }
        
        Ok(total_size)
    }
    
    /// Check if a file exists
    pub async fn file_exists<P: AsRef<Path>>(&self, file_path: P) -> bool {
        tokio::fs::metadata(file_path.as_ref()).await.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_audio_file_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = AudioFileManager::new(temp_dir.path()).unwrap();
        
        assert_eq!(manager.storage_dir, temp_dir.path());
    }
    
    #[tokio::test]
    async fn test_save_and_delete_audio_file() {
        let temp_dir = TempDir::new().unwrap();
        let manager = AudioFileManager::new(temp_dir.path()).unwrap();
        
        let audio_data = Bytes::from(vec![1, 2, 3, 4, 5]);
        let task_id = "test-task-123";
        let original_filename = "test.mp3";
        
        // Save file
        let file_path = manager.save_audio_file(task_id, &audio_data, original_filename).await.unwrap();
        
        // Check file exists
        assert!(manager.file_exists(&file_path).await);
        
        // Check file size
        let size = manager.get_file_size(&file_path).await.unwrap();
        assert_eq!(size, 5);
        
        // Delete file
        manager.delete_audio_file(&file_path).await.unwrap();
        
        // Check file no longer exists
        assert!(!manager.file_exists(&file_path).await);
    }
    
    #[tokio::test]
    async fn test_cleanup_old_files() {
        let temp_dir = TempDir::new().unwrap();
        let manager = AudioFileManager::new(temp_dir.path()).unwrap();
        
        let audio_data = Bytes::from(vec![1, 2, 3, 4, 5]);
        
        // Save a file
        let file_path = manager.save_audio_file("test-task", &audio_data, "test.mp3").await.unwrap();
        
        // File should exist
        assert!(manager.file_exists(&file_path).await);
        
        // Cleanup files older than 0 hours (should clean everything)
        let cleaned = manager.cleanup_old_files(0).await.unwrap();
        
        // Should have cleaned at least 1 file
        assert!(cleaned >= 1);
    }
    
    #[tokio::test]
    async fn test_storage_usage() {
        let temp_dir = TempDir::new().unwrap();
        let manager = AudioFileManager::new(temp_dir.path()).unwrap();
        
        // Initially should be 0
        let initial_usage = manager.get_storage_usage().await.unwrap();
        assert_eq!(initial_usage, 0);
        
        // Save a file
        let audio_data = Bytes::from(vec![1, 2, 3, 4, 5]);
        let _file_path = manager.save_audio_file("test-task", &audio_data, "test.mp3").await.unwrap();
        
        // Usage should increase
        let usage_after = manager.get_storage_usage().await.unwrap();
        assert_eq!(usage_after, 5);
    }
}