# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Testing
```bash
# Build the project
cargo build -p voice-cli

# Build in release mode
cargo build --release -p voice-cli

# Run tests (note: some integration tests may require additional setup)
cargo test -p voice-cli

# Run specific tests
cargo test test_extract_basic_metadata -p voice-cli

# Run the CLI
cargo run --bin voice-cli -- --help

# Run the server
cargo run --bin voice-cli -- server run
```

### Python Dependencies (TTS)
```bash
# Install uv package manager
curl -LsSf https://astral.sh/uv/install.sh | sh

# Install Python dependencies for TTS
uv sync

# Run TTS service directly
python3 tts_service.py --help
```

### Model Management
```bash
# List available models
cargo run --bin voice-cli -- model list

# Download a model
cargo run --bin voice-cli -- model download tiny

# Validate downloaded models
cargo run --bin voice-cli -- model validate
```

## Architecture Overview

This is a Rust-based speech-to-text HTTP service with CLI interface, built using:

- **Web Framework**: Axum for HTTP server with OpenAPI documentation
- **Speech Recognition**: Whisper models via voice-toolkit workspace dependency
- **Task Processing**: Apalis for async task queue with SQLite persistence
- **FFmpeg Integration**: ffmpeg-sidecar for lightweight media metadata extraction
- **TTS Support**: Python-based text-to-speech with uv dependency management
- **Configuration**: Multi-format config (YAML/JSON/TOML) with environment overrides

### Core Components

**Service Layer** (`src/services/`):
- `model_service.rs`: Whisper model management and downloading
- `transcription_engine.rs`: Core speech-to-text processing
- `metadata_extractor.rs`: Audio/video metadata extraction using ffmpeg-sidecar
- `tts_service.rs`: Python TTS service integration
- `apalis_manager.rs`: Async task queue management
- `audio_file_manager.rs`: File storage and management

**Server Layer** (`src/server/`):
- `handlers.rs`: HTTP request handlers for transcription and TTS
- `routes.rs`: Route definitions and OpenAPI documentation
- `middleware_config.rs`: CORS, limits, and other middleware

**Configuration** (`src/`):
- `config.rs`: Main configuration structures
- `config_rs_integration.rs`: Configuration loading with environment overrides
- `models/`: Data models for requests/responses

### Key Integrations

**FFmpeg Integration**: 
- Uses `ffmpeg-sidecar` crate for lightweight FFmpeg command execution
- Extracts audio/video metadata (duration, sample rate, codecs, etc.)
- Falls back to basic metadata extraction if FFmpeg unavailable

**TTS Integration**:
- Python-based TTS service using `tts_service.py`
- Manages Python dependencies via uv package manager
- Supports both sync and async TTS processing

**Task Queue**:
- Apalis-based async processing for transcription and TTS tasks
- SQLite persistence with task retry and cleanup mechanisms
- Supports task prioritization and status tracking

## Configuration

The service uses hierarchical configuration:
1. Default configuration values
2. Configuration file (config.yml by default)
3. Environment variables (VOICE_CLI_* prefix)
4. Command-line arguments

Key configuration sections:
- `server`: HTTP server settings (host, port, file limits)
- `whisper`: Model settings and audio processing parameters
- `task_management`: Async task processing configuration
- `tts`: Text-to-speech service configuration
- `logging`: Log levels and output settings

## Testing Notes

- Unit tests are in the same files as the code they test
- Integration tests are in `src/tests/` but may need model downloads
- Some tests may fail without proper Whisper model setup
- Use `cargo test --lib` for library tests only

## FFmpeg Dependency

The project uses `ffmpeg-sidecar` instead of heavy FFmpeg libraries:
- System FFmpeg installation required
- Uses `FfmpegCommand` for metadata extraction
- Falls back gracefully if FFmpeg unavailable