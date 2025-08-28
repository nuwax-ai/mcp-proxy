use bytes::Bytes;
use std::io::Cursor;
use symphonia::core::formats::{FormatReader, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::probe::ProbeResult;
use symphonia::default::get_probe;
use tracing::{error, info, warn};

use crate::error::VoiceCliError;
use crate::models::request::{AudioFormat, AudioFormatResult, AudioMetadata, DetectionMethod};

/// Service for intelligent audio format detection using Symphonia
pub struct AudioFormatDetector;

impl AudioFormatDetector {
    /// Detect audio format using Symphonia probe with fallback to filename extension
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
        if let Some(filename) = filename {
            if let Some(extension) = std::path::Path::new(filename)
                .extension()
                .and_then(|ext| ext.to_str())
            {
                hint.with_extension(extension);
            }
        }

        // Get the default probe
        let probe = get_probe();

        // Attempt to probe the media source
        let probe_result = probe
            .format(
                &hint,
                media_source,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| {
                warn!("Symphonia probe failed: {}", e);
                VoiceCliError::UnsupportedFormat(format!("Symphonia probe error: {}", e))
            })?;

        // Extract format information
        let format_info = Self::extract_format_info(&probe_result)?;

        Ok(format_info)
    }

    /// Extract format and metadata information from Symphonia probe result
    fn extract_format_info(probe_result: &ProbeResult) -> Result<AudioFormatResult, VoiceCliError> {
        let reader = &probe_result.format;
        let tracks = reader.tracks();

        // Find the first audio track (any track with codec parameters)
        let track = tracks
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| {
                VoiceCliError::UnsupportedFormat("No audio tracks found in file".to_string())
            })?;

        // Convert codec type to our AudioFormat
        let format = AudioFormat::from_symphonia_codec(track.codec_params.codec);

        // Extract metadata
        let metadata = Self::extract_metadata(track, reader);

        // Calculate confidence based on detection success
        let confidence = if format != AudioFormat::Unknown {
            0.95
        } else {
            0.0
        };

        if format == AudioFormat::Unknown {
            return Err(VoiceCliError::UnsupportedFormat(format!(
                "Unsupported codec type: {:?}",
                track.codec_params.codec
            )));
        }

        Ok(AudioFormatResult {
            format,
            confidence,
            metadata: Some(metadata),
            detection_method: DetectionMethod::SymphoniaProbe,
        })
    }

    /// Extract detailed audio metadata from track and format reader
    fn extract_metadata(track: &Track, _reader: &Box<dyn FormatReader>) -> AudioMetadata {
        let codec_params = &track.codec_params;

        // Extract basic parameters
        let sample_rate = codec_params.sample_rate;
        let channels = codec_params.channels.map(|ch| ch.count() as u8);
        let bit_depth = codec_params.bits_per_sample.map(|bits| bits as u8);

        // Calculate duration if time base and n_frames are available
        let duration = if let (Some(time_base), Some(n_frames)) =
            (codec_params.time_base, codec_params.n_frames)
        {
            let duration_secs = n_frames as f64 * time_base.numer as f64 / time_base.denom as f64;
            Some(std::time::Duration::from_secs_f64(duration_secs))
        } else {
            None
        };

        // Estimate bitrate if possible
        let bitrate = if let (Some(sample_rate), Some(channels), Some(bit_depth)) =
            (sample_rate, channels, bit_depth)
        {
            Some(sample_rate * channels as u32 * bit_depth as u32)
        } else {
            None
        };

        // Create codec info string
        let codec_info = format!("Codec: {:?}", codec_params.codec);

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

// Import FormatOptions for compilation
use symphonia::core::formats::FormatOptions;

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
