use crate::models::{Config, DownloadStatus, ModelDownloadStatus, ModelInfo};
use crate::VoiceCliError;
use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub struct ModelService {
    config: Config,
    client: Client,
    models_dir: PathBuf,
}

impl ModelService {
    pub fn new(config: Config) -> Self {
        Self {
            models_dir: config.models_dir_path(),
            config,
            client: Client::new(),
        }
    }

    /// Get the default model name from configuration
    pub fn default_model(&self) -> &str {
        &self.config.whisper.default_model
    }

    /// Ensure a model is available (download if necessary)
    pub async fn ensure_model(&self, model_name: &str) -> Result<(), VoiceCliError> {
        if self.is_model_downloaded(model_name).await? {
            debug!("Model '{}' already exists", model_name);
            return Ok(());
        }

        if self.config.whisper.auto_download {
            info!("Auto-downloading model: {}", model_name);
            self.download_model(model_name).await?;
        } else {
            return Err(VoiceCliError::ModelNotFound(format!(
                "Model '{}' not found and auto_download is disabled",
                model_name
            )));
        }

        Ok(())
    }

    /// Download a whisper model from the official repository
    pub async fn download_model(&self, model_name: &str) -> Result<(), VoiceCliError> {
        if !self
            .config
            .whisper
            .supported_models
            .contains(&model_name.to_string())
        {
            return Err(VoiceCliError::InvalidModelName(format!(
                "Model '{}' is not supported",
                model_name
            )));
        }

        // Create models directory if it doesn't exist
        fs::create_dir_all(&self.models_dir).await?;

        let model_path = self.get_model_path(model_name)?;

        if model_path.exists() {
            info!("Model '{}' already exists at {:?}", model_name, model_path);
            return Ok(());
        }

        info!(
            "Downloading model '{}' from whisper.cpp repository...",
            model_name
        );

        // Download from Hugging Face (official whisper.cpp models)
        let download_url = self.get_model_download_url(model_name)?;

        debug!("Download URL: {}", download_url);

        // Download with progress tracking
        let response = self
            .client
            .get(&download_url)
            .send()
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to start download: {}", e)))?;

        if !response.status().is_success() {
            return Err(VoiceCliError::Model(format!(
                "Failed to download model: HTTP {}",
                response.status()
            )));
        }

        let total_size = response.content_length().unwrap_or(0);
        info!("Downloading {} ({} bytes)...", model_name, total_size);

        // Create temporary file
        let temp_path = model_path.with_extension("tmp");
        let mut file = fs::File::create(&temp_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to create file: {}", e)))?;

        let mut downloaded = 0u64;
        let mut stream = response.bytes_stream();

        use futures::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| VoiceCliError::Model(format!("Download error: {}", e)))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| VoiceCliError::Model(format!("Failed to write file: {}", e)))?;

            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let progress = (downloaded as f32 / total_size as f32) * 100.0;
                if downloaded % (1024 * 1024) == 0 {
                    // Log every MB
                    debug!(
                        "Downloaded {:.1}% ({} / {} bytes)",
                        progress, downloaded, total_size
                    );
                }
            }
        }

        file.flush()
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to flush file: {}", e)))?;

        // Move temporary file to final location
        fs::rename(&temp_path, &model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to finalize download: {}", e)))?;

        info!(
            "Successfully downloaded model '{}' to {:?}",
            model_name, model_path
        );

        // Basic validation: just check file exists and has reasonable size
        let metadata = fs::metadata(&model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to check downloaded file: {}", e)))?;

        if metadata.len() < 1024 {
            // Clean up the invalid file
            let _ = fs::remove_file(&model_path).await;
            return Err(VoiceCliError::Model(format!(
                "Downloaded model '{}' is too small ({} bytes), likely corrupted",
                model_name,
                metadata.len()
            )));
        }

        info!(
            "Model '{}' downloaded successfully - {} bytes",
            model_name,
            metadata.len()
        );

        Ok(())
    }

    /// Get the download URL for a specific model
    fn get_model_download_url(&self, model_name: &str) -> Result<String, VoiceCliError> {
        // Whisper.cpp models are hosted on Hugging Face under ggerganov organization
        let base_url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";
        let model_filename = format!("ggml-{}.bin", model_name);
        Ok(format!("{}/{}", base_url, model_filename))
    }

    /// Get the local path for a model file
    pub fn get_model_path(&self, model_name: &str) -> Result<PathBuf, VoiceCliError> {
        let filename = format!("ggml-{}.bin", model_name);
        Ok(self.models_dir.join(filename))
    }

    /// Check if a model is downloaded locally
    pub async fn is_model_downloaded(&self, model_name: &str) -> Result<bool, VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;
        Ok(model_path.exists())
    }

    /// List all downloaded models
    pub async fn list_downloaded_models(&self) -> Result<Vec<String>, VoiceCliError> {
        if !self.models_dir.exists() {
            return Ok(Vec::new());
        }

        let mut models = Vec::new();
        let mut entries = fs::read_dir(&self.models_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                // Parse model name from filename (ggml-{model_name}.bin)
                if filename.starts_with("ggml-") && filename.ends_with(".bin") {
                    let model_name = &filename[5..filename.len() - 4]; // Remove "ggml-" and ".bin"
                    if self
                        .config
                        .whisper
                        .supported_models
                        .contains(&model_name.to_string())
                    {
                        models.push(model_name.to_string());
                    }
                }
            }
        }

        models.sort();
        Ok(models)
    }

    /// Get information about a downloaded model
    pub async fn get_model_info(&self, model_name: &str) -> Result<ModelInfo, VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;

        if !model_path.exists() {
            return Err(VoiceCliError::ModelNotFound(format!(
                "Model '{}' not found",
                model_name
            )));
        }

        let metadata = fs::metadata(&model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to get model info: {}", e)))?;

        let size = Self::format_size(metadata.len());

        // TODO: Get actual memory usage if model is loaded
        // This is a placeholder implementation - real memory tracking would require
        // integration with the transcription service to monitor loaded models
        let memory_usage = "Not tracked".to_string();

        let status = if self.is_model_valid(&model_path).await? {
            "Valid"
        } else {
            "Invalid"
        }
        .to_string();

        Ok(ModelInfo {
            size,
            memory_usage,
            status,
        })
    }

    /// Validate a downloaded model
    pub async fn validate_model(&self, model_name: &str) -> Result<(), VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;

        if !model_path.exists() {
            return Err(VoiceCliError::ModelNotFound(format!(
                "Model '{}' not found",
                model_name
            )));
        }

        if !self.is_model_valid(&model_path).await? {
            return Err(VoiceCliError::Model(format!(
                "Model '{}' validation failed",
                model_name
            )));
        }

        debug!("Model '{}' validation passed", model_name);
        Ok(())
    }

    /// Check if a model file is valid
    async fn is_model_valid(&self, model_path: &Path) -> Result<bool, VoiceCliError> {
        let metadata = fs::metadata(model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to read model file: {}", e)))?;

        // Basic validation: check if file is not empty and has reasonable size
        if metadata.len() < 1024 {
            warn!("Model file too small: {} bytes", metadata.len());
            return Ok(false);
        }

        // Check if file size is reasonable for the model type
        if let Some(expected_size) = self.get_expected_model_size(
            &model_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.strip_prefix("ggml-").unwrap_or(s))
                .unwrap_or("unknown"),
        ) {
            let actual_size = metadata.len();
            let size_diff_percent = if actual_size > expected_size {
                ((actual_size as f64 - expected_size as f64) / expected_size as f64) * 100.0
            } else {
                ((expected_size as f64 - actual_size as f64) / expected_size as f64) * 100.0
            };

            // Allow 20% size difference to accommodate different versions
            if size_diff_percent > 20.0 {
                warn!("Model file size differs significantly from expected: actual={} bytes, expected={} bytes, diff={:.1}%", 
                      actual_size, expected_size, size_diff_percent);
                // Don't fail validation, just warn - the file might still be valid
            }
        }

        // File exists and has reasonable size - assume it's valid
        // Let whisper.cpp handle format validation during actual loading
        debug!("Model file appears to be valid: {} bytes", metadata.len());
        Ok(true)
    }

    /// Remove a downloaded model
    pub async fn remove_model(&self, model_name: &str) -> Result<(), VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;

        if !model_path.exists() {
            return Ok(()); // Already removed
        }

        fs::remove_file(&model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to remove model: {}", e)))?;

        info!("Removed model '{}' from {:?}", model_name, model_path);
        Ok(())
    }

    /// Get download status for a model
    pub async fn get_download_status(
        &self,
        model_name: &str,
    ) -> Result<ModelDownloadStatus, VoiceCliError> {
        let status = if self.is_model_downloaded(model_name).await? {
            DownloadStatus::Exists
        } else {
            DownloadStatus::NotStarted
        };

        Ok(ModelDownloadStatus {
            model_name: model_name.to_string(),
            status,
            progress: None,
            message: None,
        })
    }

    /// List models that are currently loaded in memory
    pub async fn list_loaded_models(&self) -> Result<Vec<String>, VoiceCliError> {
        // TODO: This should track actually loaded models in transcription service
        // For now, return empty list as this is not a core business feature
        // Real implementation would require:
        // 1. Integration with voice-toolkit to track loaded models
        // 2. Memory usage monitoring of loaded model instances
        // 3. Reference counting for multiple concurrent uses
        Ok(Vec::new())
    }

    /// Format file size in human-readable format
    fn format_size(size: u64) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
        let mut size = size as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        format!("{:.1} {}", size, UNITS[unit_index])
    }

    /// Get the expected model size for download progress
    pub fn get_expected_model_size(&self, model_name: &str) -> Option<u64> {
        // Approximate sizes for whisper models (in bytes)
        match model_name {
            "tiny" | "tiny.en" => Some(39 * 1024 * 1024),  // ~39MB
            "base" | "base.en" => Some(142 * 1024 * 1024), // ~142MB
            "small" | "small.en" => Some(244 * 1024 * 1024), // ~244MB
            "medium" | "medium.en" => Some(769 * 1024 * 1024), // ~769MB
            "large-v1" | "large-v2" | "large-v3" => Some(1550 * 1024 * 1024), // ~1.5GB
            _ => None,
        }
    }

    /// Diagnose a corrupted model file and provide suggestions
    pub async fn diagnose_model(&self, model_name: &str) -> Result<String, VoiceCliError> {
        let model_path = self.get_model_path(model_name)?;

        if !model_path.exists() {
            return Ok(format!(
                "Model '{}' file does not exist at {:?}",
                model_name, model_path
            ));
        }

        let metadata = fs::metadata(&model_path)
            .await
            .map_err(|e| VoiceCliError::Model(format!("Failed to read model metadata: {}", e)))?;

        let mut diagnosis = Vec::new();

        // Check file size
        let actual_size = metadata.len();
        diagnosis.push(format!(
            "File size: {} bytes ({})",
            actual_size,
            Self::format_size(actual_size)
        ));

        if let Some(expected_size) = self.get_expected_model_size(model_name) {
            let size_diff = if actual_size > expected_size {
                actual_size - expected_size
            } else {
                expected_size - actual_size
            };
            let size_diff_percent = (size_diff as f64 / expected_size as f64) * 100.0;

            diagnosis.push(format!(
                "Expected size: {} bytes ({})",
                expected_size,
                Self::format_size(expected_size)
            ));
            diagnosis.push(format!("Size difference: {:.1}%", size_diff_percent));

            if size_diff_percent > 20.0 {
                diagnosis.push(
                    "⚠️  File size differs significantly from expected - may be corrupted"
                        .to_string(),
                );
            } else {
                diagnosis.push("✅ File size is within expected range".to_string());
            }
        }

        // Basic file accessibility check
        match fs::File::open(&model_path).await {
            Ok(_) => {
                diagnosis.push("✅ File is readable".to_string());
            }
            Err(e) => {
                diagnosis.push(format!("❌ File is not readable: {}", e));
            }
        }

        // Check if file is completely empty or too small
        if actual_size == 0 {
            diagnosis.push("❌ File is empty".to_string());
        } else if actual_size < 1024 {
            diagnosis.push("❌ File is too small to be a valid model".to_string());
        } else {
            diagnosis.push("✅ File has reasonable size".to_string());
        }

        Ok(diagnosis.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_model_service_creation() {
        let config = Config::default();
        let service = ModelService::new(config);
        assert!(!service.models_dir.as_os_str().is_empty());
    }

    #[tokio::test]
    async fn test_model_path_generation() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.whisper.models_dir = temp_dir.path().to_string_lossy().to_string();

        let service = ModelService::new(config);
        let path = service.get_model_path("base").unwrap();

        assert!(path.to_string_lossy().contains("ggml-base.bin"));
    }

    #[tokio::test]
    async fn test_list_downloaded_models_empty() {
        let temp_dir = TempDir::new().unwrap();
        let mut config = Config::default();
        config.whisper.models_dir = temp_dir.path().to_string_lossy().to_string();

        let service = ModelService::new(config);
        let models = service.list_downloaded_models().await.unwrap();

        assert!(models.is_empty());
    }

    #[test]
    fn test_format_size() {
        assert_eq!(ModelService::format_size(1024), "1.0 KB");
        assert_eq!(ModelService::format_size(1024 * 1024), "1.0 MB");
        assert_eq!(ModelService::format_size(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn test_get_expected_model_size() {
        let service = ModelService::new(Config::default());

        assert!(service.get_expected_model_size("base").is_some());
        assert!(service.get_expected_model_size("unknown").is_none());
    }
}
