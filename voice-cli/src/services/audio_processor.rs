use crate::models::{AudioFormat, ProcessedAudio};
use crate::VoiceCliError;
use bytes::Bytes;
use std::path::PathBuf;
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::{info, debug, warn};

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
        debug!("Processing audio data: {} bytes, filename: {:?}", audio_data.len(), filename);
        
        // Detect audio format
        let format = self.detect_audio_format(&audio_data, filename)?;
        debug!("Detected audio format: {:?}", format);
        
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
    fn detect_audio_format(&self, audio_data: &Bytes, filename: Option<&str>) -> Result<AudioFormat, VoiceCliError> {
        // First try to detect from filename extension
        if let Some(filename) = filename {
            let format = AudioFormat::from_filename(filename);
            if format.is_supported() {
                return Ok(format);
            }
        }
        
        // Try to detect from file headers (magic bytes)
        if audio_data.len() >= 4 {
            let header = &audio_data[0..4];
            
            // WAV file signature
            if header == b"RIFF" && audio_data.len() >= 12 {
                let wave_header = &audio_data[8..12];
                if wave_header == b"WAVE" {
                    return Ok(AudioFormat::Wav);
                }
            }
            
            // MP3 file signatures
            if header[0..3] == [0xFF, 0xFB, 0x90] || // MP3 frame header
               header[0..3] == [0x49, 0x44, 0x33] {  // ID3 tag
                return Ok(AudioFormat::Mp3);
            }
            
            // FLAC file signature
            if header == b"fLaC" {
                return Ok(AudioFormat::Flac);
            }
            
            // OGG file signature
            if header == b"OggS" {
                return Ok(AudioFormat::Ogg);
            }
        }
        
        // Check for M4A/AAC (more complex detection)
        if audio_data.len() >= 8 {
            let ftyp_check = &audio_data[4..8];
            if ftyp_check == b"ftyp" {
                return Ok(AudioFormat::M4a);
            }
        }
        
        // Default to unknown if we can't detect
        Err(VoiceCliError::UnsupportedFormat("Unable to detect audio format".to_string()))
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
        let output_file = NamedTempFile::new_in(&self.temp_dir)
            .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to create temp output file: {}", e)))?;
        
        let input_path = input_file.path();
        let output_path = output_file.path();
        
        // Try to use rs-voice-toolkit for conversion
        match self.convert_with_rs_voice_toolkit(input_path, output_path).await {
            Ok(_) => {
                // Read converted file
                let converted_data = std::fs::read(output_path)
                    .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to read converted file: {}", e)))?;
                
                info!("Successfully converted audio: {} -> {} bytes", audio_data.len(), converted_data.len());
                Ok(Bytes::from(converted_data))
            }
            Err(toolkit_error) => {
                warn!("rs-voice-toolkit conversion failed: {}, trying fallback", toolkit_error);
                
                // Fallback to FFmpeg if available
                self.convert_with_ffmpeg(input_path, output_path).await
                    .and_then(|_| {
                        let converted_data = std::fs::read(output_path)
                            .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to read converted file: {}", e)))?;
                        Ok(Bytes::from(converted_data))
                    })
                    .map_err(|ffmpeg_error| {
                        VoiceCliError::AudioProcessing(format!(
                            "Both rs-voice-toolkit and FFmpeg conversion failed. Toolkit: {}, FFmpeg: {}", 
                            toolkit_error, ffmpeg_error
                        ))
                    })
            }
        }
    }

    /// Convert using rs-voice-toolkit
    async fn convert_with_rs_voice_toolkit(&self, input_path: &std::path::Path, output_path: &std::path::Path) -> Result<(), VoiceCliError> {
        // This is a placeholder - actual implementation would use rs-voice-toolkit
        // We'll use a simplified approach for now since the actual API might differ
        
        // For now, we'll create a simple wrapper that calls the expected rs-voice-toolkit functions
        // This should be replaced with actual rs-voice-toolkit calls once we have the exact API
        
        use std::process::Command;
        
        // Try to use rs-voice-toolkit's command line interface if available
        let output = Command::new("rs-voice-toolkit-cli")
            .args(&[
                "convert",
                "--input", input_path.to_str().unwrap(),
                "--output", output_path.to_str().unwrap(),
                "--format", "wav",
                "--sample-rate", "16000",
                "--channels", "1",
                "--bit-depth", "16"
            ])
            .output();
        
        match output {
            Ok(result) => {
                if result.status.success() {
                    Ok(())
                } else {
                    let error_msg = String::from_utf8_lossy(&result.stderr);
                    Err(VoiceCliError::AudioProcessing(format!("rs-voice-toolkit conversion failed: {}", error_msg)))
                }
            }
            Err(e) => {
                Err(VoiceCliError::AudioProcessing(format!("Failed to execute rs-voice-toolkit: {}", e)))
            }
        }
    }

    /// Fallback conversion using FFmpeg
    async fn convert_with_ffmpeg(&self, input_path: &std::path::Path, output_path: &std::path::Path) -> Result<(), VoiceCliError> {
        use std::process::Command;
        
        let output = Command::new("ffmpeg")
            .args(&[
                "-i", input_path.to_str().unwrap(),
                "-ar", "16000",           // Sample rate: 16kHz
                "-ac", "1",               // Channels: mono
                "-sample_fmt", "s16",     // Sample format: 16-bit signed
                "-y",                     // Overwrite output file
                output_path.to_str().unwrap()
            ])
            .output();
        
        match output {
            Ok(result) => {
                if result.status.success() {
                    Ok(())
                } else {
                    let error_msg = String::from_utf8_lossy(&result.stderr);
                    Err(VoiceCliError::AudioProcessing(format!("FFmpeg conversion failed: {}", error_msg)))
                }
            }
            Err(e) => {
                Err(VoiceCliError::AudioProcessing(format!("FFmpeg not available: {}", e)))
            }
        }
    }

    /// Create temporary file with audio data
    fn create_temp_file(&self, audio_data: &Bytes, format: &AudioFormat) -> Result<NamedTempFile, VoiceCliError> {
        let extension = format.to_string();
        let mut temp_file = NamedTempFile::with_suffix_in(&format!(".{}", extension), &self.temp_dir)
            .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to create temp file: {}", e)))?;
        
        temp_file.write_all(audio_data)
            .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to write temp file: {}", e)))?;
        
        temp_file.flush()
            .map_err(|e| VoiceCliError::AudioProcessing(format!("Failed to flush temp file: {}", e)))?;
        
        Ok(temp_file)
    }

    /// Validate that the processed audio is in the correct format for Whisper
    pub fn validate_whisper_format(&self, audio_data: &Bytes) -> Result<(), VoiceCliError> {
        // Basic WAV header validation
        if audio_data.len() < 44 {
            return Err(VoiceCliError::AudioProcessing("Audio file too small to be valid WAV".to_string()));
        }
        
        let header = &audio_data[0..44];
        
        // Check RIFF header
        if &header[0..4] != b"RIFF" {
            return Err(VoiceCliError::AudioProcessing("Invalid WAV file: missing RIFF header".to_string()));
        }
        
        // Check WAVE format
        if &header[8..12] != b"WAVE" {
            return Err(VoiceCliError::AudioProcessing("Invalid WAV file: missing WAVE format".to_string()));
        }
        
        // Check fmt chunk
        if &header[12..16] != b"fmt " {
            return Err(VoiceCliError::AudioProcessing("Invalid WAV file: missing fmt chunk".to_string()));
        }
        
        // Extract audio parameters
        let sample_rate = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
        let channels = u16::from_le_bytes([header[22], header[23]]);
        let bits_per_sample = u16::from_le_bytes([header[34], header[35]]);
        
        debug!("WAV format - Sample rate: {}Hz, Channels: {}, Bits: {}", sample_rate, channels, bits_per_sample);
        
        // Whisper prefers 16kHz, mono, 16-bit, but can handle other formats too
        // We'll just warn for non-optimal formats rather than error
        if sample_rate != 16000 {
            warn!("Non-optimal sample rate: {}Hz (Whisper prefers 16kHz)", sample_rate);
        }
        
        if channels != 1 {
            warn!("Non-optimal channel count: {} (Whisper prefers mono)", channels);
        }
        
        if bits_per_sample != 16 {
            warn!("Non-optimal bit depth: {} (Whisper prefers 16-bit)", bits_per_sample);
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
        assert!(matches!(processor.detect_audio_format(&wav_data, Some("test.wav")), Ok(AudioFormat::Wav)));
        
        // Test MP3 detection from filename
        let mp3_data = Bytes::from_static(b"\xFF\xFB\x90\x00");
        assert!(matches!(processor.detect_audio_format(&mp3_data, Some("test.mp3")), Ok(AudioFormat::Mp3)));
    }
}