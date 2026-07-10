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

### TTS (sherpa-onnx Kokoro / ZipVoice)
> TTS 默认禁用（`config.tts.enabled=false`）。启用需在 `config.yml` 置 `tts.enabled: true`
> 并放置模型目录（见下）。`config.tts.engine.backend`：`kokoro`（标准多音色，默认）/ `zipvoice`（零样本克隆，中英）。
> 缺模型时 `/api/v1/tts*` 返回明确错误，不阻塞 STT。

**模型放置**（HuggingFace 阻断时用 gh-proxy 或 modelscope；可翻墙则去掉 gh-proxy 前缀直连）：
```bash
mkdir -p ./models/tts
# Kokoro（标准多音色，103 音色）
curl -SL https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_1.tar.bz2 \
  -o /tmp/kokoro.tar.bz2
tar xjf /tmp/kokoro.tar.bz2 -C ./models/tts
# 目录布局：./models/tts/kokoro-multi-lang-v1_1/{model.onnx,voices.bin,tokens.txt,espeak-ng-data/,dict/,lexicon-*.txt}

# ZipVoice（零样本克隆，中英）+ vocoder（独立下载）
curl -SL https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-zipvoice-distill-int8-zh-en-emilia.tar.bz2 \
  -o /tmp/zipvoice.tar.bz2 && tar xjf /tmp/zipvoice.tar.bz2 -C ./models/tts
curl -SL https://github.com/k2-fsa/sherpa-onnx/releases/download/vocoder-models/vocos_24khz.onnx \
  -o ./models/tts/zipvoice-distill-int8-zh-en-emilia/vocos_24khz.onnx
# 布局：./models/tts/zipvoice-distill-int8-zh-en-emilia/{encoder.int8.onnx,decoder.int8.onnx,tokens.txt,espeak-ng-data/,lexicon.txt,test_wavs/}
# 克隆参考音频：预置 voices[]（请求 voice:<name>）或请求期 reference_audio(base64 WAV)。配置见 deploy/config.example.yml
```

**编译**（sherpa-onnx-sys 需预编译 C 库；github 阻断时设 `SHERPA_ONNX_ARCHIVE_DIR`）：
```bash
export SHERPA_ONNX_ARCHIVE_DIR="$HOME/.cache/sherpa-onnx-prebuilt"  # 预下载的 tar.bz2 所在目录
cargo build -p voice-cli
```

**TTS 接口**（重新设计的 `/api/v1/` 风格，旧 `/tts/sync` 已移除）：
- `POST /api/v1/tts`：同步合成，返回 wav/pcm_s16le 二进制
- `GET  /api/v1/tts/voices`：查询音色数（`num_speakers`）
- `POST /api/v1/tasks/tts` + `GET /{id}` + `GET /{id}/audio`：异步任务管线
- `GET   /api/v1/stream/tts`（WebSocket）：流式合成，客户端发 `{type:"start",text,...}`，
  服务端推 `ready` → 二进制 PCM s16le 增量帧 → `done`

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
- **TTS Support**: sherpa-onnx TTS — Kokoro（标准多音色）/ ZipVoice（零样本克隆，中英）
- **Configuration**: Multi-format config (YAML/JSON/TOML) with environment overrides

### Core Components

**Service Layer** (`src/services/`):
- `model_service.rs`: Whisper model management and downloading
- `metadata_extractor.rs`: Audio/video metadata extraction using ffmpeg-sidecar
- `tts_apalis_manager.rs`: TTS async task queue (apalis + SQLite, mirrors `apalis_manager.rs`)
- `apalis_manager.rs`: STT async task queue management
- `audio_file_manager.rs`: File storage and management

**TTS Library** (`src/tts/`): sherpa-onnx Kokoro / ZipVoice 双引擎池 + 合成（镜像 STT `src/stt/` + fastembed ModelPool）
- `engine_pool.rs`: 进程级 OfflineTts 引擎池（DashMap + double-checked + round-robin；build_engine 按 backend 分派 Kokoro/ZipVoice）
- `reference_profiles.rs`: ZipVoice 克隆参考管理（预置 profile 缓存 + 动态 base64 WAV 解码，Wave::read）
- `synthesizer.rs`: generate_with_config 封装（NUL 预清洗 + Option→TtsError；reference 注入 GenerationConfig）
- `streaming.rs`: 流式合成（callback → mpsc 增量 PCM）
- `audio_encode.rs`: f32 → WAV / PCM s16le
- `model_service.rs`: Kokoro / ZipVoice 模型目录解析（TtsModelPaths 枚举分引擎）

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

**TTS Integration** (sherpa-onnx Kokoro, CPU v1):
- `OfflineTts` 引擎池 + 合成（`src/tts/`），Kokoro multi-lang v1.0（53 音色）
- 同步 `/api/v1/tts` + 异步 `/api/v1/tasks/tts` + 流式 WS `/api/v1/stream/tts`
- 编译需 `SHERPA_ONNX_ARCHIVE_DIR`（sherpa-onnx-sys 预编译 C 库，见 docs/DEPLOYMENT.md）

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