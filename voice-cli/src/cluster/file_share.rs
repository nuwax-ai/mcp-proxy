use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::fs;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;
use bytes::Bytes;
use serde::{Serialize, Deserialize};

use crate::models::{ClusterError, TaskMetadata};

/// File sharing strategy for cluster task distribution
#[derive(Debug, Clone)]
pub enum FileShareStrategy {
    /// Shared network file system (NFS, CIFS, etc.)
    SharedFileSystem { mount_path: PathBuf },
    /// Object storage (S3, MinIO, etc.)
    ObjectStorage { bucket: String, endpoint: String },
    /// Distributed file replication
    DistributedReplication { replication_factor: u32 },
    /// Local storage with HTTP transfer
    HttpTransfer { base_url: String },
}

/// File metadata for cluster sharing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedFileInfo {
    pub file_id: String,
    pub original_filename: String,
    pub file_size: u64,
    pub content_type: String,
    pub checksum: String,
    pub storage_path: String,
    pub created_at: SystemTime,
    pub accessed_at: SystemTime,
    pub expires_at: Option<SystemTime>,
    pub available_nodes: Vec<String>,
}

/// File sharing configuration
#[derive(Debug, Clone)]
pub struct FileShareConfig {
    /// Strategy for file sharing
    pub strategy: FileShareStrategy,
    /// Base directory for local file storage
    pub storage_base_dir: PathBuf,
    /// Maximum file size for sharing
    pub max_file_size: u64,
    /// File time-to-live (TTL) for cleanup
    pub file_ttl: Duration,
    /// Replication timeout for distributed strategies
    pub replication_timeout: Duration,
    /// Enable file compression for transfer
    pub enable_compression: bool,
    /// Chunk size for large file transfers
    pub chunk_size: usize,
}

impl Default for FileShareConfig {
    fn default() -> Self {
        Self {
            strategy: FileShareStrategy::SharedFileSystem {
                mount_path: PathBuf::from("./shared-voice-cli"),
            },
            storage_base_dir: PathBuf::from("./cluster-storage"),
            max_file_size: 500 * 1024 * 1024, // 500MB
            file_ttl: Duration::from_secs(3600), // 1 hour
            replication_timeout: Duration::from_secs(30),
            enable_compression: false,
            chunk_size: 1024 * 1024, // 1MB chunks
        }
    }
}

/// Comprehensive file sharing manager for cluster task distribution
pub struct ClusterFileShare {
    /// Configuration
    config: FileShareConfig,
    /// Shared file registry
    shared_files: Arc<RwLock<HashMap<String, SharedFileInfo>>>,
    /// Current node ID
    node_id: String,
    /// File share manager ID
    manager_id: String,
}

impl ClusterFileShare {
    /// Create a new cluster file share manager
    pub async fn new(config: FileShareConfig, node_id: String) -> Result<Self, ClusterError> {
        info!("Initializing cluster file share with strategy: {:?}", config.strategy);

        // Ensure storage directory exists
        if let Err(e) = fs::create_dir_all(&config.storage_base_dir).await {
            return Err(ClusterError::Config(format!(
                "Failed to create storage directory: {}", e
            )));
        }

        Ok(Self {
            config,
            shared_files: Arc::new(RwLock::new(HashMap::new())),
            node_id,
            manager_id: Uuid::new_v4().to_string(),
        })
    }

    /// Store an audio file for cluster sharing
    pub async fn store_audio_file(
        &self,
        task_id: &str,
        filename: &str,
        audio_data: Bytes,
    ) -> Result<SharedFileInfo, ClusterError> {
        info!("Storing audio file for task {} ({})", task_id, filename);

        // Validate file size
        if audio_data.len() as u64 > self.config.max_file_size {
            return Err(ClusterError::InvalidOperation(format!(
                "File size {} exceeds maximum {}", 
                audio_data.len(), 
                self.config.max_file_size
            )));
        }

        let file_id = format!("{}_{}", task_id, Uuid::new_v4().simple());
        let checksum = self.calculate_checksum(&audio_data);
        
        // Determine storage strategy and store file
        let storage_path = match &self.config.strategy {
            FileShareStrategy::SharedFileSystem { mount_path } => {
                self.store_to_shared_filesystem(&file_id, &audio_data, mount_path).await?
            }
            FileShareStrategy::ObjectStorage { bucket, endpoint } => {
                self.store_to_object_storage(&file_id, &audio_data, bucket, endpoint).await?
            }
            FileShareStrategy::DistributedReplication { replication_factor } => {
                self.store_with_replication(&file_id, &audio_data, *replication_factor).await?
            }
            FileShareStrategy::HttpTransfer { base_url } => {
                self.store_for_http_transfer(&file_id, &audio_data, base_url).await?
            }
        };

        // Create file info
        let file_info = SharedFileInfo {
            file_id: file_id.clone(),
            original_filename: filename.to_string(),
            file_size: audio_data.len() as u64,
            content_type: self.detect_content_type(filename),
            checksum,
            storage_path,
            created_at: SystemTime::now(),
            accessed_at: SystemTime::now(),
            expires_at: Some(SystemTime::now() + self.config.file_ttl),
            available_nodes: vec![self.node_id.clone()],
        };

        // Register the file
        {
            let mut shared_files = self.shared_files.write().await;
            shared_files.insert(file_id.clone(), file_info.clone());
        }

        info!("Successfully stored file {} for task {}", file_id, task_id);
        Ok(file_info)
    }

    /// Retrieve an audio file for task processing
    pub async fn retrieve_audio_file(
        &self,
        file_id: &str,
    ) -> Result<Bytes, ClusterError> {
        info!("Retrieving audio file {}", file_id);

        // Get file info
        let file_info = {
            let shared_files = self.shared_files.read().await;
            shared_files.get(file_id).cloned()
        };

        let file_info = file_info.ok_or_else(|| {
            ClusterError::InvalidOperation(format!("File {} not found", file_id))
        })?;

        // Check if file has expired
        if let Some(expires_at) = file_info.expires_at {
            if SystemTime::now() > expires_at {
                warn!("File {} has expired", file_id);
                return Err(ClusterError::InvalidOperation(format!("File {} has expired", file_id)));
            }
        }

        // Retrieve file based on storage strategy
        let audio_data = match &self.config.strategy {
            FileShareStrategy::SharedFileSystem { .. } => {
                self.retrieve_from_shared_filesystem(&file_info.storage_path).await?
            }
            FileShareStrategy::ObjectStorage { bucket, endpoint } => {
                self.retrieve_from_object_storage(&file_info.storage_path, bucket, endpoint).await?
            }
            FileShareStrategy::DistributedReplication { .. } => {
                self.retrieve_from_replicas(&file_info.storage_path).await?
            }
            FileShareStrategy::HttpTransfer { base_url } => {
                self.retrieve_via_http_transfer(&file_info.storage_path, base_url).await?
            }
        };

        // Update access time
        {
            let mut shared_files = self.shared_files.write().await;
            if let Some(info) = shared_files.get_mut(file_id) {
                info.accessed_at = SystemTime::now();
            }
        }

        // Verify checksum
        let retrieved_checksum = self.calculate_checksum(&audio_data);
        if retrieved_checksum != file_info.checksum {
            return Err(ClusterError::InvalidOperation(format!(
                "File {} checksum mismatch: expected {}, got {}", 
                file_id, file_info.checksum, retrieved_checksum
            )));
        }

        info!("Successfully retrieved file {} ({} bytes)", file_id, audio_data.len());
        Ok(audio_data)
    }

    /// Clean up expired files
    pub async fn cleanup_expired_files(&self) -> Result<u32, ClusterError> {
        let now = SystemTime::now();
        let mut expired_files = Vec::new();

        // Find expired files
        {
            let shared_files = self.shared_files.read().await;
            for (file_id, file_info) in shared_files.iter() {
                if let Some(expires_at) = file_info.expires_at {
                    if now > expires_at {
                        expired_files.push((file_id.clone(), file_info.clone()));
                    }
                }
            }
        }

        let cleanup_count = expired_files.len() as u32;
        
        // Remove expired files
        for (file_id, file_info) in expired_files {
            if let Err(e) = self.remove_file(&file_id, &file_info.storage_path).await {
                warn!("Failed to remove expired file {}: {}", file_id, e);
            } else {
                // Remove from registry
                let mut shared_files = self.shared_files.write().await;
                shared_files.remove(&file_id);
                debug!("Removed expired file {}", file_id);
            }
        }

        if cleanup_count > 0 {
            info!("Cleaned up {} expired files", cleanup_count);
        }

        Ok(cleanup_count)
    }

    /// Get file sharing statistics
    pub async fn get_statistics(&self) -> FileShareStatistics {
        let shared_files = self.shared_files.read().await;
        
        let total_files = shared_files.len();
        let total_size: u64 = shared_files.values().map(|f| f.file_size).sum();
        let expired_count = shared_files.values()
            .filter(|f| f.expires_at.map_or(false, |exp| SystemTime::now() > exp))
            .count();

        FileShareStatistics {
            total_files,
            total_size_bytes: total_size,
            expired_files: expired_count,
            node_id: self.node_id.clone(),
            manager_id: self.manager_id.clone(),
            strategy: format!("{:?}", self.config.strategy),
        }
    }

    // Storage strategy implementations

    /// Store file to shared filesystem
    async fn store_to_shared_filesystem(
        &self,
        file_id: &str,
        audio_data: &Bytes,
        mount_path: &Path,
    ) -> Result<String, ClusterError> {
        let file_path = mount_path.join(format!("{}.audio", file_id));
        
        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await
                .map_err(|e| ClusterError::Network(format!("Failed to create directory: {}", e)))?;
        }

        // Write file
        fs::write(&file_path, audio_data).await
            .map_err(|e| ClusterError::Network(format!("Failed to write file: {}", e)))?;

        Ok(file_path.to_string_lossy().to_string())
    }

    /// Store file to object storage (simplified implementation)
    async fn store_to_object_storage(
        &self,
        file_id: &str,
        audio_data: &Bytes,
        _bucket: &str,
        _endpoint: &str,
    ) -> Result<String, ClusterError> {
        // For now, use local storage as fallback
        // In a real implementation, this would use S3/MinIO SDK
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        fs::write(&local_path, audio_data).await
            .map_err(|e| ClusterError::Network(format!("Failed to write to object storage: {}", e)))?;
        
        Ok(format!("object://{}", file_id))
    }

    /// Store file with distributed replication
    async fn store_with_replication(
        &self,
        file_id: &str,
        audio_data: &Bytes,
        _replication_factor: u32,
    ) -> Result<String, ClusterError> {
        // For now, use local storage as primary
        // In a real implementation, this would replicate to multiple nodes
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        fs::write(&local_path, audio_data).await
            .map_err(|e| ClusterError::Network(format!("Failed to write for replication: {}", e)))?;
        
        Ok(format!("replicated://{}", file_id))
    }

    /// Store file for HTTP transfer
    async fn store_for_http_transfer(
        &self,
        file_id: &str,
        audio_data: &Bytes,
        _base_url: &str,
    ) -> Result<String, ClusterError> {
        // Store locally for HTTP serving
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        fs::write(&local_path, audio_data).await
            .map_err(|e| ClusterError::Network(format!("Failed to write for HTTP transfer: {}", e)))?;
        
        Ok(format!("http://{}", file_id))
    }

    // Retrieval strategy implementations

    /// Retrieve file from shared filesystem
    async fn retrieve_from_shared_filesystem(&self, storage_path: &str) -> Result<Bytes, ClusterError> {
        let data = fs::read(storage_path).await
            .map_err(|e| ClusterError::Network(format!("Failed to read from shared filesystem: {}", e)))?;
        Ok(Bytes::from(data))
    }

    /// Retrieve file from object storage
    async fn retrieve_from_object_storage(
        &self,
        storage_path: &str,
        _bucket: &str,
        _endpoint: &str,
    ) -> Result<Bytes, ClusterError> {
        // Extract file ID from storage path
        let file_id = storage_path.strip_prefix("object://")
            .ok_or_else(|| ClusterError::InvalidOperation("Invalid object storage path".to_string()))?;
        
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        let data = fs::read(local_path).await
            .map_err(|e| ClusterError::Network(format!("Failed to read from object storage: {}", e)))?;
        Ok(Bytes::from(data))
    }

    /// Retrieve file from replicas
    async fn retrieve_from_replicas(&self, storage_path: &str) -> Result<Bytes, ClusterError> {
        // Extract file ID from storage path
        let file_id = storage_path.strip_prefix("replicated://")
            .ok_or_else(|| ClusterError::InvalidOperation("Invalid replication path".to_string()))?;
        
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        let data = fs::read(local_path).await
            .map_err(|e| ClusterError::Network(format!("Failed to read from replicas: {}", e)))?;
        Ok(Bytes::from(data))
    }

    /// Retrieve file via HTTP transfer
    async fn retrieve_via_http_transfer(&self, storage_path: &str, _base_url: &str) -> Result<Bytes, ClusterError> {
        // Extract file ID from storage path
        let file_id = storage_path.strip_prefix("http://")
            .ok_or_else(|| ClusterError::InvalidOperation("Invalid HTTP path".to_string()))?;
        
        let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
        let data = fs::read(local_path).await
            .map_err(|e| ClusterError::Network(format!("Failed to read for HTTP transfer: {}", e)))?;
        Ok(Bytes::from(data))
    }

    /// Remove a file from storage
    async fn remove_file(&self, file_id: &str, storage_path: &str) -> Result<(), ClusterError> {
        match &self.config.strategy {
            FileShareStrategy::SharedFileSystem { .. } => {
                fs::remove_file(storage_path).await
                    .map_err(|e| ClusterError::Network(format!("Failed to remove file: {}", e)))?;
            }
            _ => {
                // For other strategies, remove local copy
                let local_path = self.config.storage_base_dir.join(format!("{}.audio", file_id));
                if local_path.exists() {
                    fs::remove_file(local_path).await
                        .map_err(|e| ClusterError::Network(format!("Failed to remove local file: {}", e)))?;
                }
            }
        }
        Ok(())
    }

    /// Calculate file checksum
    fn calculate_checksum(&self, data: &Bytes) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Detect content type from filename
    fn detect_content_type(&self, filename: &str) -> String {
        let extension = Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav", 
            "flac" => "audio/flac",
            "m4a" => "audio/mp4",
            "aac" => "audio/aac",
            "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        }.to_string()
    }
}

/// File sharing statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileShareStatistics {
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub expired_files: usize,
    pub node_id: String,
    pub manager_id: String,
    pub strategy: String,
}

/// Enhanced task distribution with real file sharing
pub struct TaskDistributor {
    /// File share manager
    file_share: Arc<ClusterFileShare>,
    /// Node ID
    #[allow(dead_code)]
    node_id: String,
}

impl TaskDistributor {
    /// Create a new task distributor
    pub fn new(file_share: Arc<ClusterFileShare>, node_id: String) -> Self {
        Self {
            file_share,
            node_id,
        }
    }

    /// Distribute a task with real file sharing
    pub async fn distribute_task_with_file_sharing(
        &self,
        task_metadata: &mut TaskMetadata,
        audio_data: Bytes,
    ) -> Result<String, ClusterError> {
        info!("Distributing task {} with real file sharing", task_metadata.task_id);

        // Store the audio file for cluster access
        let file_info = self.file_share.store_audio_file(
            &task_metadata.task_id,
            &task_metadata.filename,
            audio_data,
        ).await?;

        // Update task metadata with file path
        task_metadata.audio_file_path = Some(file_info.storage_path.clone());

        info!("Task {} file stored successfully at {}", 
              task_metadata.task_id, file_info.storage_path);

        Ok(file_info.file_id)
    }

    /// Retrieve audio data for task processing
    pub async fn retrieve_task_audio_data(
        &self,
        file_id: &str,
    ) -> Result<Bytes, ClusterError> {
        self.file_share.retrieve_audio_file(file_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_file_share_creation() {
        let temp_dir = tempdir().unwrap();
        let config = FileShareConfig {
            storage_base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let file_share = ClusterFileShare::new(config, "test-node".to_string()).await;
        assert!(file_share.is_ok());
    }

    #[tokio::test]
    async fn test_audio_file_storage_and_retrieval() {
        let temp_dir = tempdir().unwrap();
        let config = FileShareConfig {
            storage_base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let file_share = ClusterFileShare::new(config, "test-node".to_string()).await.unwrap();
        
        // Test data
        let task_id = "test-task-123";
        let filename = "test.wav";
        let audio_data = Bytes::from(vec![1, 2, 3, 4, 5]);

        // Store file
        let file_info = file_share.store_audio_file(task_id, filename, audio_data.clone()).await.unwrap();
        assert!(!file_info.file_id.is_empty());
        assert_eq!(file_info.original_filename, filename);
        assert_eq!(file_info.file_size, 5);

        // Retrieve file
        let retrieved_data = file_share.retrieve_audio_file(&file_info.file_id).await.unwrap();
        assert_eq!(retrieved_data, audio_data);
    }

    #[tokio::test]
    async fn test_task_distributor() {
        let temp_dir = tempdir().unwrap();
        let config = FileShareConfig {
            storage_base_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let file_share = Arc::new(ClusterFileShare::new(config, "test-node".to_string()).await.unwrap());
        let distributor = TaskDistributor::new(file_share, "test-node".to_string());

        let mut task_metadata = TaskMetadata::new(
            "test-task".to_string(),
            "test-client".to_string(),
            "test.wav".to_string(),
        );

        let audio_data = Bytes::from(vec![1, 2, 3, 4, 5]);

        // Distribute task
        let file_id = distributor.distribute_task_with_file_sharing(&mut task_metadata, audio_data.clone()).await.unwrap();
        assert!(!file_id.is_empty());
        assert!(task_metadata.audio_file_path.is_some());

        // Retrieve audio data
        let retrieved_data = distributor.retrieve_task_audio_data(&file_id).await.unwrap();
        assert_eq!(retrieved_data, audio_data);
    }
}