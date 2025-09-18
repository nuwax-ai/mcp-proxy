use crate::models::request::ProcessedAudio;
use crate::models::AudioFormat;
use crate::VoiceCliError;
use bytes::Bytes;
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tracing::{debug, info, warn};

#[derive(Debug)]
pub struct AudioProcessor {
    temp_dir: PathBuf,
}

impl AudioProcessor {
    pub fn new(temp_dir: Option<PathBuf>) -> Self {
        let temp_dir = temp_dir.unwrap_or_else(|| std::env::temp_dir().join("voice-cli"));

        // Ensure temp directory exists
        if let Err(e) = std::fs::create_dir_all(&temp_dir) {
            warn!("Failed to create temp directory {:?}: {}", temp_dir, e);
        }

        Self { temp_dir }
    }

    /// Process audio data and convert to whisper-compatible format if needed
    pub async fn process_audio(
        &self,
        audio_data: Bytes,
        filename: Option<&str>,
    ) -> Result<ProcessedAudio, VoiceCliError> {
        debug!(
            "Processing audio data: {} bytes, filename: {:?}",
            audio_data.len(),
            filename
        );

        // Detect audio format
        let format = self.detect_audio_format(&audio_data, filename)?;
        debug!("Detected audio format: {:?}", format);

        // For WAV format, validate the content since it doesn't need conversion
        if format == AudioFormat::Wav {
            self.validate_whisper_format(&audio_data)?;
        }

        // Check if conversion is needed
        if !format.needs_conversion() {
            debug!("Audio format is already compatible, no conversion needed");
            return Ok(ProcessedAudio {
                data: audio_data,
                converted: false,
                original_format: Some(format.to_string().to_string()),
            });
        }

        // Convert to Whisper-compatible format
        info!("Converting audio from {} to WAV format", format.to_string());
        let converted_data = self.convert_to_whisper_format(audio_data, format).await?;

        Ok(ProcessedAudio {
            data: converted_data,
            converted: true,
            original_format: Some(format.to_string().to_string()),
        })
    }

    /// Detect audio format from data and filename
    fn detect_audio_format(
        &self,
        audio_data: &Bytes,
        filename: Option<&str>,
    ) -> Result<AudioFormat, VoiceCliError> {
        // Use AudioFormatDetector for enhanced detection
        use crate::services::AudioFormatDetector;

        match AudioFormatDetector::detect_format(audio_data, filename) {
            Ok(format_result) => {
                // Validate format support
                AudioFormatDetector::validate_format_support(&format_result)?;
                Ok(format_result.format)
            }
            Err(e) => Err(e),
        }
    }

    /// Convert audio to Whisper-compatible format (16kHz, mono, 16-bit PCM WAV)
    async fn convert_to_whisper_format(
        &self,
        audio_data: Bytes,
        source_format: AudioFormat,
    ) -> Result<Bytes, VoiceCliError> {
        debug!("Converting {} to WAV format", source_format.to_string());

        // Create temporary files for input and output
        let input_file = self.create_temp_file(&audio_data, &source_format)?;
        let output_file = NamedTempFile::new_in(&self.temp_dir).map_err(|e| {
            VoiceCliError::AudioProcessing(format!("Failed to create temp output file: {}", e))
        })?;

        let input_path = input_file.path();
        let output_path = output_file.path();

        // Try to use rs-voice-toolkit for conversion
        match self
            .convert_with_rs_voice_toolkit(input_path, output_path)
            .await
        {
            Ok(_) => {
                // Read converted file
                let converted_data = std::fs::read(output_path).map_err(|e| {
                    VoiceCliError::AudioProcessing(format!("Failed to read converted file: {}", e))
                })?;

                info!(
                    "Successfully converted audio: {} -> {} bytes",
                    audio_data.len(),
                    converted_data.len()
                );
                Ok(Bytes::from(converted_data))
            }
            Err(toolkit_error) => {
                warn!(
                    "rs-voice-toolkit conversion failed: {}, trying fallback",
                    toolkit_error
                );

                return Err(toolkit_error);
            }
        }
    }

    /// Convert using rs-voice-toolkit
    async fn convert_with_rs_voice_toolkit(
        &self,
        input_path: &std::path::Path,
        output_path: &std::path::Path,
    ) -> Result<(), VoiceCliError> {
        debug!(
            "Converting audio using voice-toolkit: {:?} -> {:?}",
            input_path, output_path
        );

        // Use voice_toolkit::audio::ensure_whisper_compatible for real audio conversion
        match voice_toolkit::audio::ensure_whisper_compatible(
            input_path,
            Some(output_path.to_path_buf()),
        ) {
            Ok(compatible_wav) => {
                debug!(
                    "Successfully converted audio to whisper-compatible format: {:?}",
                    compatible_wav.path
                );

                // Verify the output file exists and is at the expected location
                if compatible_wav.path != output_path {
                    // If the output is in a different location, move it to the expected location
                    if let Err(e) = std::fs::rename(&compatible_wav.path, output_path) {
                        warn!("Failed to move converted file to expected location: {}", e);
                        // Try copying instead
                        std::fs::copy(&compatible_wav.path, output_path).map_err(|e| {
                            VoiceCliError::AudioProcessing(format!(
                                "Failed to copy converted file: {}",
                                e
                            ))
                        })?;
                        // Clean up the original if copy succeeded
                        let _ = std::fs::remove_file(&compatible_wav.path);
                    }
                }

                info!("Audio conversion completed successfully");
                Ok(())
            }
            Err(e) => {
                warn!("voice-toolkit conversion failed: {}", e);
                Err(VoiceCliError::AudioProcessing(format!(
                    "Audio conversion failed: {}",
                    e
                )))
            }
        }
    }

    /// Create temporary file with audio data
    fn create_temp_file(
        &self,
        audio_data: &Bytes,
        format: &AudioFormat,
    ) -> Result<NamedTempFile, VoiceCliError> {
        let extension = format.to_string();
        let mut temp_file =
            NamedTempFile::with_suffix_in(&format!(".{}", extension), &self.temp_dir).map_err(
                |e| VoiceCliError::AudioProcessing(format!("Failed to create temp file: {}", e)),
            )?;

        temp_file.write_all(audio_data).map_err(|e| {
            VoiceCliError::AudioProcessing(format!("Failed to write temp file: {}", e))
        })?;

        temp_file.flush().map_err(|e| {
            VoiceCliError::AudioProcessing(format!("Failed to flush temp file: {}", e))
        })?;

        Ok(temp_file)
    }

    /// Validate that the processed audio is in the correct format for Whisper
    pub fn validate_whisper_format(&self, audio_data: &Bytes) -> Result<(), VoiceCliError> {
        // Basic WAV header validation
        if audio_data.len() < 44 {
            return Err(VoiceCliError::AudioProcessing(
                "Audio file too small to be valid WAV".to_string(),
            ));
        }

        let header = &audio_data[0..44];

        // Check RIFF header
        if &header[0..4] != b"RIFF" {
            return Err(VoiceCliError::AudioProcessing(
                "Invalid WAV file: missing RIFF header".to_string(),
            ));
        }

        // Check WAVE format
        if &header[8..12] != b"WAVE" {
            return Err(VoiceCliError::AudioProcessing(
                "Invalid WAV file: missing WAVE format".to_string(),
            ));
        }

        // Check fmt chunk
        if &header[12..16] != b"fmt " {
            return Err(VoiceCliError::AudioProcessing(
                "Invalid WAV file: missing fmt chunk".to_string(),
            ));
        }

        // Extract audio parameters
        let sample_rate = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
        let channels = u16::from_le_bytes([header[22], header[23]]);
        let bits_per_sample = u16::from_le_bytes([header[34], header[35]]);

        debug!(
            "WAV format - Sample rate: {}Hz, Channels: {}, Bits: {}",
            sample_rate, channels, bits_per_sample
        );

        // Whisper prefers 16kHz, mono, 16-bit, but can handle other formats too
        // We'll just warn for non-optimal formats rather than error
        if sample_rate != 16000 {
            warn!(
                "Non-optimal sample rate: {}Hz (Whisper prefers 16kHz)",
                sample_rate
            );
        }

        if channels != 1 {
            warn!(
                "Non-optimal channel count: {} (Whisper prefers mono)",
                channels
            );
        }

        if bits_per_sample != 16 {
            warn!(
                "Non-optimal bit depth: {} (Whisper prefers 16-bit)",
                bits_per_sample
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_audio_processor_creation() {
        let temp_dir = TempDir::new().unwrap();
        let processor = AudioProcessor::new(Some(temp_dir.path().to_path_buf()));

        // Test with mock WAV data (basic WAV header)
        let wav_header = b"RIFF\x24\x00\x00\x00WAVE";
        let audio_data = Bytes::from_static(wav_header);

        let format = processor.detect_audio_format(&audio_data, Some("test.wav"));
        assert!(matches!(format, Ok(AudioFormat::Wav)));
    }

    #[test]
    fn test_audio_format_detection() {
        let processor = AudioProcessor::new(None);

        // Test WAV detection from filename
        let wav_data = Bytes::from_static(b"RIFF\x24\x00\x00\x00WAVE");
        assert!(matches!(
            processor.detect_audio_format(&wav_data, Some("test.wav")),
            Ok(AudioFormat::Wav)
        ));

        // Test MP3 detection from filename
        let mp3_data = Bytes::from_static(b"\xFF\xFB\x90\x00");
        assert!(matches!(
            processor.detect_audio_format(&mp3_data, Some("test.mp3")),
            Ok(AudioFormat::Mp3)
        ));
    }
}
