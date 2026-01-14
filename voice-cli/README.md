# Voice CLI - Speech-to-Text Service

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# Voice CLI - Speech-to-Text Service

High-performance speech-to-text HTTP service built with Rust, leveraging Whisper engine for accurate speech recognition.

## Core Features

### Speech Processing Capabilities
- **Multi-Format Support**: MP3, WAV, FLAC, M4A, AAC, OGG and other mainstream audio formats
- **Automatic Format Conversion**: Intelligent audio processing via rs-voice-toolkit
- **Whisper Models**: Support for tiny/base/small/medium/large model series
- **Automatic Model Management**: On-demand Whisper model download and management

### Deployment Modes
- **🏠 Single-Node Deployment**: Quick start, suitable for small-scale usage

### Advanced Features
- **RESTful API**: Complete HTTP API interface
- **Real-time Monitoring**: Service status, health checks
- **Task Processing**: Efficient audio processing pipeline

> **Note**: TTS (Text-to-Speech) feature is currently under development and has known issues. It will be available in a future release.

## System Requirements

- **Operating System**: Linux/macOS/Windows
- **Memory**: Minimum 2GB, recommended 8GB+
- **Storage**: At least 5GB (for model storage)

## Quick Installation

### Build from Source

```bash
# Clone the project
git clone https://github.com/nuwax-ai/mcp-proxy
cd mcp-proxy

# Build voice-cli
cargo build --release -p voice-cli

# Binary location
ls target/release/voice-cli
```

## Single-Node Deployment

### Method 1: Direct Run (Simplest)

```bash
# 1. Switch to working directory
mkdir -p /opt/voice-service
cd /opt/voice-service
cp /path/to/voice-cli ./

# 2. Start service (auto-create config file)
./voice-cli server run

# 3. Test service
curl -X POST http://localhost:8080/transcribe \
  -F "audio=@test.mp3" \
  -F "model=base"
```

### Method 2: Background Run (using nohup)

```bash
# 1. Start background service
nohup ./voice-cli server run > server.log 2>&1 &

# 2. Check process status
ps aux | grep voice-cli

# 3. Stop service
pkill -f "voice-cli server run"

# 4. View logs
tail -f server.log
```

### Method 3: System Service (Recommended for Production)

```bash
# 1. Create configuration file
./voice-cli server init

# 2. Edit configuration file (optional)
nano server-config.yml

# 3. Run with configuration file
./voice-cli server run --config server-config.yml
```

## Configuration

### Generate Configuration File

```bash
# Generate default configuration file
./voice-cli server init

# Generate to specified path
./voice-cli server init --config /path/to/config.yml

# Force overwrite existing file
./voice-cli server init --force
```

### Configuration File Example (server-config.yml)

```yaml
server:
  host: "0.0.0.0"
  port: 8080
  max_file_size: 268435456  # 256MB
  cors_enabled: true

whisper:
  default_model: "base"
  models_dir: "./models"
  auto_download: true
  supported_models:
    - "tiny"
    - "base"
    - "small"
    - "medium"
    - "large"
  audio_processing:
    sample_rate: 16000
    channels: 1
    bit_depth: 16
  workers:
    transcription_workers: 2
    channel_buffer_size: 100
    worker_timeout: 3600

logging:
  level: "info"
  log_dir: "./logs"
  max_file_size: "10MB"
  max_files: 10

daemon:
  pid_file: "./voice_cli.pid"
  log_file: "./logs/daemon.log"
  work_dir: "./work"
```

## Command Line Usage

### Main Commands

```bash
# Initialize configuration file
voice-cli server init [--config <path>] [--force]

# Run service (foreground mode)
voice-cli server run [--config <path>]

# Display help information
voice-cli --help
voice-cli server --help
```

### Environment Variable Configuration

Override configuration via environment variables:

```bash
# HTTP port
VOICE_CLI_PORT=8081

# Log level
VOICE_CLI_LOG_LEVEL=debug

# Default model
VOICE_CLI_DEFAULT_MODEL=large

# Model directory
VOICE_CLI_MODELS_DIR=/opt/models
```

## HTTP API

### Speech Transcription Endpoint

```bash
POST /transcribe
Content-Type: multipart/form-data

Parameters:
- audio: Audio file (required)
- model: Model name (optional, default uses configured default model)
- language: Language code (optional, e.g., "zh", "en")
- response_format: Response format (optional, "json" or "text", default "json")
```

### Health Check Endpoint

```bash
GET /health
```

### Example Request

```bash
# Using curl
curl -X POST http://localhost:8080/transcribe \
  -F "audio=@speech.wav" \
  -F "model=base" \
  -F "language=zh"

# Response example
{
  "text": "Hello, this is a test speech",
  "language": "zh",
  "duration": 5.2,
  "model": "base",
  "processing_time": 2.1
}
```

## Monitoring and Logging

### Log Files
- `./logs/server.log` - Service running logs
- `./logs/daemon.log` - Background service logs

### Log Levels
Supports the following log levels:
- `trace` - Most detailed debug information
- `debug` - Debug information
- `info` - General information (default)
- `warn` - Warning information
- `error` - Error information

## Troubleshooting

### Common Issues

1. **Port Already in Use**
   ```bash
   # Check port occupation
   lsof -i :8080

   # Kill occupying process
   kill -9 <PID>

   # Or modify configured port
   VOICE_CLI_PORT=8081 ./voice-cli server run
   ```

2. **Model Download Failed**
   ```bash
   # Check network connection
   curl -I https://huggingface.co

   # Manual model download
   # Models downloaded to ./models/ggml-{model_name}.bin
   ```

3. **Insufficient Memory**
   ```bash
   # Use smaller model
   VOICE_CLI_DEFAULT_MODEL=tiny ./voice-cli server run

   # Reduce worker threads
   VOICE_CLI_TRANSCRIPTION_WORKERS=1 ./voice-cli server run
   ```

### Debug Mode

```bash
# Enable verbose logging
RUST_LOG=debug ./voice-cli server run

# View real-time logs
tail -f ./logs/server.log
```

## License

This project is licensed under MIT License. See [LICENSE](LICENSE) file for details.

## Contributing

Issues and Pull Requests are welcome!

## Support

- Submit Issues: [GitHub Issues](https://github.com/nuwax-ai/mcp-proxy/issues)
- Documentation: [Project Wiki](https://github.com/nuwax-ai/mcp-proxy/wiki)
