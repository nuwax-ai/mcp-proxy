use crate::services::ModelService;
use crate::VoiceCliError;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use dashmap::DashMap;

// Reuse an already-loaded WhisperTranscriber to avoid reloading the model
use voice_toolkit::stt::{self, TranscriptionResult, WhisperConfig, WhisperTranscriber};

/// Shared transcription engine to unify model resolution, audio conversion and transcription
pub struct TranscriptionEngine {
    model_service: Arc<ModelService>,
    // Cache transcribers per model to avoid reloading model/VRAM each time
    // Using DashMap for better concurrent performance
    transcribers: DashMap<String, Arc<WhisperTranscriber>>,
}

impl std::fmt::Debug for TranscriptionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranscriptionEngine")
            .field("model_service", &self.model_service)
            .field("transcribers_count", &self.transcribers.len())
            .finish()
    }
}

impl TranscriptionEngine {
    /// Create a new transcription engine
    pub fn new(model_service: Arc<ModelService>) -> Self {
        Self {
            model_service,
            transcribers: DashMap::new(),
        }
    }

    async fn get_or_create_transcriber(
        &self,
        model_name: &str,
    ) -> Result<Arc<WhisperTranscriber>, VoiceCliError> {
        // Fast path: try get from cache
        if let Some(existing) = self.transcribers.get(model_name) {
            return Ok(existing.clone());
        }

        // Resolve model path
        let model_path = self.model_service.get_model_path(model_name)?;
        if !model_path.exists() {
            return Err(VoiceCliError::ModelNotFound(model_name.to_string()));
        }

        // Create transcriber (assume construction might be CPU-heavy)
        let created_res = tokio::task::spawn_blocking(move || {
            let cfg = WhisperConfig::new(model_path);
            WhisperTranscriber::new(cfg)
        })
        .await
        .map_err(|e| VoiceCliError::Model(format!("Transcriber create join error: {}", e)))?;

        let created = created_res.map_err(|e| VoiceCliError::Model(e.to_string()))?;
        let transcriber = Arc::new(created);

        // Insert into cache using DashMap's atomic operations
        // Use entry API to handle race conditions where another thread might have inserted the same key
        match self.transcribers.entry(model_name.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => {
                // Another thread already inserted this transcriber, use the existing one
                Ok(entry.get().clone())
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                // We're the first to insert, use our transcriber
                entry.insert(transcriber.clone());
                Ok(transcriber)
            }
        }
    }

    /// Transcribe an audio file that is already Whisper-compatible (wav, correct params)
    pub async fn transcribe_compatible_audio(
        &self,
        model_name: &str,
        audio_path: &Path,
        timeout_secs: u64,
    ) -> Result<TranscriptionResult, VoiceCliError> {
        let transcriber = self.get_or_create_transcriber(model_name).await?;
        let audio_path = audio_path.to_path_buf();

        let timeout_duration = std::time::Duration::from_secs(timeout_secs);
        let result = tokio::time::timeout(
            timeout_duration,
            stt::transcribe_file_with_transcriber(&transcriber, &audio_path),
        )
        .await
        .map_err(|_| VoiceCliError::TranscriptionTimeout(timeout_secs))?
        .map_err(|e| VoiceCliError::TranscriptionFailed(e.to_string()))?;

        Ok(result)
    }

    /// Get the default model name from configuration
    pub fn default_model(&self) -> &str {
        self.model_service.default_model()
    }

    /// Get the worker timeout from configuration
    pub fn worker_timeout(&self) -> u64 {
        self.model_service.worker_timeout()
    }

    /// Transcribe an input audio file, converting to Whisper-compatible format if necessary
    pub async fn transcribe_with_conversion(
        &self,
        model_name: &str,
        input_audio_path: &Path,
        timeout_secs: u64,
    ) -> Result<TranscriptionResult, VoiceCliError> {
        // Convert to Whisper-compatible format in blocking thread
        let input_path = input_audio_path.to_path_buf();
        let compatible = tokio::task::spawn_blocking(move || {
            voice_toolkit::audio::ensure_whisper_compatible(&input_path, None::<PathBuf>)
        })
        .await
        .map_err(|e| VoiceCliError::AudioConversionFailed(format!("Task join error: {}", e)))?
        .map_err(|e| VoiceCliError::AudioConversionFailed(e.to_string()))?;

        self
            .transcribe_compatible_audio(model_name, &compatible.path, timeout_secs)
            .await
    }
}


