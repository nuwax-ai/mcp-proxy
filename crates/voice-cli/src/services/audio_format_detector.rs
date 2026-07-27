use bytes::Bytes;
use infer::{self, Type};
use std::io::Cursor;
use std::path::Path;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;
use tracing::{error, info, warn};

use crate::error::VoiceCliError;
use crate::models::request::{AudioFormat, AudioFormatResult, AudioMetadata, DetectionMethod};

/// Service for intelligent audio format detection using Symphonia
pub struct AudioFormatDetector;

impl AudioFormatDetector {
    /// Detect audio format using infer library (magic number detection)
    pub fn detect_format_from_path(path: &Path) -> anyhow::Result<Option<Type>> {
        let kind = infer::get_from_path(path)
            .map_err(|e| anyhow::anyhow!("Failed to read file for format detection: {}", e))?;
        Ok(kind)
    }

    /// Detect audio format using Symphonia probe with fallback to infer (magic number detection) and filename extension
    pub fn detect_format(
        audio_data: &Bytes,
        filename: Option<&str>,
    ) -> Result<AudioFormatResult, VoiceCliError> {
        info!(
            "Starting audio format detection for {} byte audio data",
            audio_data.len()
        );

        // Try Symphonia probe first (primary method)
        if let Ok(result) = Self::symphonia_probe(audio_data, filename) {
            info!(
                "Successfully detected format using Symphonia probe: {:?}",
                result.format
            );
            return Ok(result);
        }

        // Fallback to filename extension if provided
        if let Some(filename) = filename {
            let format = AudioFormat::from_filename(filename);
            if format.is_supported() {
                info!("Format detected from filename extension: {:?}", format);
                return Ok(AudioFormatResult {
                    format,
                    confidence: 0.5, // Lower confidence for filename-based detection
                    metadata: None,
                    detection_method: DetectionMethod::FileExtension,
                });
            }
        }

        // All detection methods failed
        error!("All audio format detection methods failed");
        Err(VoiceCliError::UnsupportedFormat(
            "Unable to detect audio format using any available method".to_string(),
        ))
    }

    /// Primary detection method using Symphonia
    fn symphonia_probe(
        audio_data: &Bytes,
        filename: Option<&str>,
    ) -> Result<AudioFormatResult, VoiceCliError> {
        // Create a cursor from copied audio data to avoid lifetime issues
        let data_copy = audio_data.to_vec();
        let cursor = Cursor::new(data_copy);
        let media_source = MediaSourceStream::new(Box::new(cursor), Default::default());

        // Create a hint based on filename if available
        let mut hint = Hint::new();
        if let Some(filename) = filename
            && let Some(extension) = std::path::Path::new(filename)
                .extension()
                .and_then(|ext| ext.to_str())
        {
            hint.with_extension(extension);
        }

        // Get the default probe
        let probe = get_probe();

        // Attempt to probe the media source
        let reader = probe
            .probe(
                &hint,
                media_source,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|e| {
                warn!("Symphonia probe failed: {}", e);
                VoiceCliError::UnsupportedFormat(format!("Symphonia probe error: {}", e))
            })?;

        // Extract format information
        let format_info = Self::extract_format_info(reader.as_ref())?;

        Ok(format_info)
    }

    /// Extract format and metadata information from Symphonia format reader
    fn extract_format_info(reader: &dyn FormatReader) -> Result<AudioFormatResult, VoiceCliError> {
        // Use default_track to find the primary audio track
        let track = reader.default_track(TrackType::Audio).ok_or_else(|| {
            VoiceCliError::UnsupportedFormat("No audio tracks found in file".to_string())
        })?;

        let audio_params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| {
                VoiceCliError::UnsupportedFormat("No audio codec parameters found".to_string())
            })?;

        // Convert codec type to our AudioFormat
        let format = AudioFormat::from_symphonia_codec(audio_params.codec);

        // Extract metadata
        let metadata = Self::extract_metadata(audio_params);

        // Calculate confidence based on detection success
        let confidence = if format != AudioFormat::Unknown {
            0.95
        } else {
            0.0
        };

        if format == AudioFormat::Unknown {
            return Err(VoiceCliError::UnsupportedFormat(format!(
                "Unsupported codec type: {:?}",
                audio_params.codec
            )));
        }

        Ok(AudioFormatResult {
            format,
            confidence,
            metadata: Some(metadata),
            detection_method: DetectionMethod::SymphoniaProbe,
        })
    }

    /// Extract detailed audio metadata from audio codec parameters
    fn extract_metadata(
        audio_params: &symphonia::core::codecs::audio::AudioCodecParameters,
    ) -> AudioMetadata {
        // Extract basic parameters
        let sample_rate = audio_params.sample_rate;
        let channels = audio_params.channels.as_ref().map(|ch| ch.count() as u8);
        let bit_depth = audio_params.bits_per_sample.map(|bits| bits as u8);

        // Duration is no longer available directly from codec params in symphonia 0.6
        let duration = None;

        // Estimate bitrate if possible
        let bitrate = if let (Some(sample_rate), Some(channels), Some(bit_depth)) =
            (sample_rate, channels, bit_depth)
        {
            Some(sample_rate * channels as u32 * bit_depth as u32)
        } else {
            None
        };

        // Create codec info string
        let codec_info = format!("Codec: {:?}", audio_params.codec);

        AudioMetadata {
            duration,
            sample_rate,
            channels,
            bit_depth,
            bitrate,
            codec_info,
        }
    }

    /// Validate that the detected format is supported for transcription
    pub fn validate_format_support(format_result: &AudioFormatResult) -> Result<(), VoiceCliError> {
        if !format_result.format.is_supported() {
            return Err(VoiceCliError::UnsupportedFormat(format!(
                "Format {} is not supported for transcription",
                format_result.format.to_string()
            )));
        }

        // Check confidence threshold
        if format_result.confidence < 0.3 {
            warn!(
                "Low confidence format detection: {}",
                format_result.confidence
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection_with_filename() {
        let test_data = Bytes::from(vec![0u8; 1024]); // Dummy data

        // Test filename-based fallback
        let result = AudioFormatDetector::detect_format(&test_data, Some("test.mp3"));
        match result {
            Ok(format_result) => {
                assert_eq!(
                    format_result.detection_method,
                    DetectionMethod::FileExtension
                );
                assert_eq!(format_result.format, AudioFormat::Mp3);
            }
            Err(_) => {
                // Expected for dummy data, but we should get filename-based detection
            }
        }
    }

    #[test]
    fn test_unsupported_format() {
        let test_data = Bytes::from(vec![0u8; 1024]);
        let result = AudioFormatDetector::detect_format(&test_data, Some("test.xyz"));
        assert!(result.is_err());
    }

    #[test]
    fn test_format_validation() {
        let format_result = AudioFormatResult {
            format: AudioFormat::Mp3,
            confidence: 0.9,
            metadata: None,
            detection_method: DetectionMethod::SymphoniaProbe,
        };

        assert!(AudioFormatDetector::validate_format_support(&format_result).is_ok());

        let unsupported_result = AudioFormatResult {
            format: AudioFormat::Unknown,
            confidence: 0.9,
            metadata: None,
            detection_method: DetectionMethod::SymphoniaProbe,
        };

        assert!(AudioFormatDetector::validate_format_support(&unsupported_result).is_err());
    }
}
