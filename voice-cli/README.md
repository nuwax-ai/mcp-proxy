# Voice CLI

Speech-to-text HTTP service with CLI interface built on top of the mcp_proxy workspace.

## Features

- **HTTP API**: RESTful API for audio transcription
- **Multiple Audio Formats**: Support for MP3, WAV, FLAC, M4A, AAC, OGG
- **Automatic Conversion**: Audio format conversion using rs-voice-toolkit
- **Model Management**: Automatic download and management of whisper.cpp models
- **Daemon Mode**: Background server operation with process management
- **CLI Interface**: Comprehensive command-line tools for server and model management

## Quick Start

### Installation

```bash
# Build from source
cargo build --release -p voice-cli

# The binary will be available at target/release/voice-cli
```

### Basic Usage

1. **Start the server** (auto-generates config.yml if not exists):
   ```bash
   ./voice-cli server run
   ```

2. **Download a model** (if not auto-downloaded):
   ```bash
   ./voice-cli model download base
   ```

3. **Transcribe audio**:
   ```bash
   curl -X POST http://localhost:8080/transcribe \
     -F "audio=@your_audio.mp3" \
     -F "model=base" \
     -F "language=en"
   ```

## CLI Commands

### Server Management

```bash
# Run server in foreground
voice-cli server run

# Start server in background (daemon)
voice-cli server start

# Stop background server
voice-cli server stop

# Restart background server
voice-cli server restart

# Check server status
voice-cli server status
```

### Model Management

```bash
# Download a specific model
voice-cli model download base

# List available and downloaded models
voice-cli model list

# Validate downloaded models
voice-cli model validate

# Remove a downloaded model
voice-cli model remove base
```

## Configuration

The service uses a `config.yml` file that is automatically generated with sensible defaults:

```yaml
server:
  host: "0.0.0.0"
  port: 8080
  max_file_size: 209715200  # 200MB
  cors_enabled: true

whisper:
  default_model: "base"
  models_dir: "./models"
  auto_download: true
  supported_models:
    - "tiny"
    - "tiny.en"
    - "base"
    - "base.en"
    - "small"
    - "small.en"
    - "medium"
    - "medium.en"
    - "large-v1"
    - "large-v2"
    - "large-v3"

logging:
  level: "info"
  log_dir: "./logs"
  max_file_size: "10MB"
  max_files: 5

daemon:
  pid_file: "./voice-cli.pid"
  log_file: "./logs/daemon.log"
  work_dir: "./"
```

## API Endpoints

### POST /transcribe

Convert audio file to text.

**Request:**
- Content-Type: `multipart/form-data`
- Max file size: 200MB (configurable)

**Form Fields:**
- `audio` (file, required): Audio file to transcribe
- `model` (text, optional): Whisper model to use
- `language` (text, optional): Language hint for better accuracy
- `response_format` (text, optional): Output format (json, text, verbose_json)

**Response:**
```json
{
    "text": "Hello, this is a test transcription.",
    "segments": [
        {
            "start": 0.0,
            "end": 2.5,
            "text": "Hello, this is a test transcription.",
            "confidence": 0.95
        }
    ],
    "language": "en",
    "duration": 2.5,
    "processing_time": 0.8
}
```

### GET /health

Health check endpoint.

**Response:**
```json
{
    "status": "healthy",
    "models_loaded": ["base"],
    "uptime": 3600,
    "version": "0.1.0"
}
```

### GET /models

List available and loaded models.

**Response:**
```json
{
    "available_models": ["tiny", "base", "small", "medium", "large"],
    "loaded_models": ["base"],
    "model_info": {
        "base": {
            "size": "142 MB",
            "memory_usage": "388 MB",
            "status": "loaded"
        }
    }
}
```

## Audio Format Support

The service automatically detects and converts audio formats:

- **Input formats**: MP3, WAV, FLAC, M4A, AAC, OGG
- **Whisper format**: 16kHz, mono, 16-bit PCM WAV (automatic conversion)
- **Max file size**: 200MB (configurable)

## Model Information

Whisper models are automatically downloaded from the official repository:

| Model | Size | Languages | Description |
|-------|------|-----------|-------------|
| tiny | ~39 MB | English/Multilingual | Fastest, lowest accuracy |
| base | ~142 MB | English/Multilingual | Good balance of speed/accuracy |
| small | ~244 MB | English/Multilingual | Better accuracy |
| medium | ~769 MB | English/Multilingual | High accuracy |
| large | ~1.5 GB | Multilingual only | Best accuracy |

## Dependencies

- **rs-voice-toolkit**: Audio processing and STT capabilities
- **whisper.cpp**: Underlying speech recognition engine
- **Axum**: HTTP server framework
- **Tokio**: Async runtime

## Development

### Building

```bash
# Build the project
cargo build -p voice-cli

# Run tests
cargo test -p voice-cli

# Build with release optimizations
cargo build --release -p voice-cli
```

### Testing

```bash
# Run all tests
cargo test -p voice-cli

# Run with output
cargo test -p voice-cli -- --nocapture

# Test specific module
cargo test -p voice-cli services::
```

## Troubleshooting

### Common Issues

1. **Port already in use**: Change the port in `config.yml` or stop the conflicting service
2. **Model download fails**: Check internet connection and disk space
3. **Audio conversion fails**: Ensure FFmpeg is installed as fallback
4. **Permission denied**: Check file permissions for models and logs directories

### Logs

- **Console logs**: Real-time output when running in foreground
- **File logs**: `./logs/voice-cli.log` with daily rotation
- **Daemon logs**: `./logs/daemon.log` for background mode

### Debug Mode

```bash
# Enable verbose logging
voice-cli --verbose server run

# Check configuration
voice-cli --config custom.yml server status
```

## License

This project is part of the mcp_proxy workspace and follows the same licensing terms.