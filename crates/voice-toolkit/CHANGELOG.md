# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2024-01-20

### Added
- Initial release of rs-voice-toolkit
- **STT (Speech-to-Text) Module**:
  - Whisper-based transcription with file and streaming support
  - Voice Activity Detection (VAD) integration
  - Async processing with configurable parameters
  - Performance baseline testing and metrics
- **TTS (Text-to-Speech) Module**:
  - Index-TTS engine integration
  - Configurable synthesis parameters
  - Memory and file output options
- **Audio Processing Module**:
  - Audio format conversion and resampling
  - Whisper-compatible audio preprocessing
  - Audio metadata extraction
- **Streaming Transcription**:
  - Real-time audio processing
  - Configurable chunk sizes and overlap
  - VAD-based silence detection
- **Integration & Performance Tests**:
  - End-to-end workflow testing
  - Performance benchmarking with RTF metrics
  - Memory usage monitoring
- **Documentation**:
  - Comprehensive usage guides for all modules
  - Performance optimization recommendations
  - Streaming configuration best practices

### Technical Details
- Rust 2021 edition
- Async/await support throughout
- Modular architecture with separate crates
- Cross-platform compatibility (macOS, Linux, Windows)
- MIT/Apache-2.0 dual licensing

[Unreleased]: https://github.com/your-org/rs-voice-toolkit/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/your-org/rs-voice-toolkit/releases/tag/v0.1.0