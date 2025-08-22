use crate::models::{Config, TranscriptionResponse};
use crate::models::worker::TranscriptionRequest;
use crate::VoiceCliError;
use std::sync::Arc;
use tracing::info;

/// Simple transcription service that handles validation and delegates to worker pool
pub struct TranscriptionService {
    config: Arc<Config>,
}

impl TranscriptionService {
    pub async fn new(config: Arc<Config>) -> Result<Self, VoiceCliError> {
        Ok(Self {
            config,
        })
    }

    /// Validate transcription request parameters
    pub fn validate_request(&self, request: &TranscriptionRequest) -> Result<(), VoiceCliError> {
        // Validate model name if provided
        if let Some(ref model_name) = request.model {
            if !self.config.whisper.supported_models.contains(model_name) {
                return Err(VoiceCliError::InvalidModelName(
                    format!("Unsupported model: {}", model_name)
                ));
            }
        }

        // Validate response format if provided
        if let Some(ref format) = request.response_format {
            match format.as_str() {
                "json" | "text" | "verbose_json" => {},
                _ => {
                    return Err(VoiceCliError::AudioProcessing(
                        format!("Unsupported response format: {}", format)
                    ));
                }
            }
        }

        Ok(())
    }

    /// Get the model name to use for transcription
    pub fn get_model_name(&self, request: &TranscriptionRequest) -> String {
        request.model
            .as_ref()
            .unwrap_or(&self.config.whisper.default_model)
            .clone()
    }

    /// Check if audio format conversion is needed
    pub fn needs_conversion(&self, filename: &str) -> bool {
        use crate::models::AudioFormat;
        let format = AudioFormat::from_filename(filename);
        !matches!(format, AudioFormat::Wav)
    }

    /// Get list of supported audio formats
    pub fn get_supported_formats(&self) -> Vec<String> {
        self.config.whisper.audio_processing.supported_formats.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Config;

    #[tokio::test]
    async fn test_transcription_service_creation() {
        let config = Arc::new(Config::default());
        let service = TranscriptionService::new(config).await;
        assert!(service.is_ok());
    }

    #[test]
    fn test_request_validation() {
        let config = Arc::new(Config::default());
        let service = TranscriptionService {
            config,
        };
        
        let request = TranscriptionRequest {
            filename: "test.wav".to_string(),
            model: Some("invalid_model".to_string()),
            response_format: None,
        };
        
        let result = service.validate_request(&request);
        assert!(result.is_err());
    }
}