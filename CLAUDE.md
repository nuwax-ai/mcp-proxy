# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

### Building and Testing
```bash
# Build all workspace crates
cargo build

# Build specific crate
cargo build -p mcp-proxy
cargo build -p voice-cli
cargo build -p document-parser
cargo build -p oss-client

# Build in release mode
cargo build --release

# Run tests for all crates
cargo test

# Run tests for specific crate
cargo test -p mcp-proxy
cargo test -p voice-cli

# Run clippy for linting
cargo clippy --all-targets --all-features

# Format code
cargo fmt

# Check formatting
cargo fmt --check
```

### Cross-Platform Building (using Docker)
```bash
# Build document-parser for Linux x86_64
make build-document-parser-x86_64

# Build document-parser for Linux ARM64
make build-document-parser-arm64

# Build voice-cli for Linux x86_64
make build-voice-cli-x86_64

# Build all components for x86_64
make build-all-x86_64

# Build Docker runtime image
make build-image

# Run Docker container
make run
```

### Service-Specific Commands

**Document Parser:**
```bash
# Initialize Python environment
cd document-parser && cargo run --bin document-parser -- uv-init

# Check environment status
cd document-parser && cargo run --bin document-parser -- check

# Start server
cd document-parser && cargo run --bin document-parser -- server

# Troubleshoot issues
cd document-parser && cargo run --bin document-parser -- troubleshoot
```

**Voice CLI:**
```bash
# Initialize server configuration
cd voice-cli && cargo run --bin voice-cli -- server init

# Run voice server
cd voice-cli && cargo run --bin voice-cli -- server run

# List Whisper models
cd voice-cli && cargo run --bin voice-cli -- model list

# Download model
cd voice-cli && cargo run --bin voice-cli -- model download tiny
```

**MCP Proxy:**
```bash
# Start MCP proxy server
cd mcp-proxy && cargo run --bin mcp-proxy
```

## Architecture Overview

This is a Rust workspace implementing an MCP (Model Context Protocol) proxy system with multiple services:

### Core Services

**mcp-proxy**: Main MCP proxy service implementing SSE protocol
- Provides HTTP API for MCP service management
- Handles dynamic plugin loading and configuration
- Implements Server-Sent Events for real-time communication
- Uses `rmcp` crate for MCP protocol implementation

**document-parser**: High-performance document parsing service
- Multi-format support: PDF, Word, Excel, PowerPoint via MinerU/MarkItDown
- Python-based with uv dependency management
- GPU acceleration support with CUDA
- Automatic virtual environment management

**voice-cli**: Speech-to-text service with TTS capabilities
- Whisper model integration for transcription
- Python-based TTS service with uv management
- Apalis-based async task queue
- FFmpeg integration for metadata extraction

**oss-client**: Lightweight Alibaba Cloud OSS client
- Simple interface for object storage operations
- Workspace dependency for other services

### Key Integrations

**Workspace Management**: All dependencies managed through workspace Cargo.toml with centralized versioning. Sub-crates use `{ workspace = true }` for dependency references.

**Async Processing**: Uses `tokio` runtime throughout. Task processing via `apalis` with SQLite persistence for voice transcription and TTS tasks.

**HTTP Framework**: `axum` with `tower` middleware for all web services. OpenAPI documentation via `utoipa`.

**Error Handling**: Consistent error handling with `anyhow` for application code and `thiserror` for library code.

**Logging**: Structured logging with `tracing` and `tracing-subscriber`. Daily log rotation with `tracing-appender`.

**FFmpeg Integration**: Lightweight FFmpeg command execution via `ffmpeg-sidecar` for media metadata extraction. System FFmpeg installation required but provides graceful fallback.

**Python Integration**: Both `document-parser` and `voice-cli` use Python services with `uv` for dependency management and virtual environment handling:
- Automatic virtual environment creation in `./venv/`
- uv package manager for fast Python dependency installation
- CUDA GPU acceleration support (optional)
- Graceful degradation if Python/uv unavailable

**Task Queue & Persistence**: Voice services use `apalis` for background task processing:
- SQLite-based persistence for task state tracking
- Task retry mechanisms with exponential backoff
- Support for task prioritization and status monitoring
- Worker management with resource limits

### Configuration System

All services use hierarchical configuration:
1. Default values in code
2. Configuration files (YAML/JSON/TOML)
3. Environment variables with service prefixes
4. Command-line arguments

### Development Standards

**Code Organization**: Strict workspace structure - no code in root directory. All implementation in sub-crates with clear module boundaries.

**Formatting & Linting**: 
- Line length: 100 characters
- 4-space indentation (no tabs)
- Always run `cargo fmt` and `cargo clippy` before commits
- Use `cargo audit` to check for security vulnerabilities
- Use `typos-cli` to check spelling

**Error Handling**: 
- Prefer `anyhow` for application code, `thiserror` for libraries
- Avoid `unwrap()` except in tests
- Use `?` operator for error propagation
- Add contextual error messages with `anyhow::Context`
- Never include sensitive data in error messages

**Concurrency**: 
- Use `tokio` for async, `Arc<Mutex<T>>` or `Arc<RwLock<T>>` for shared state
- Avoid blocking operations in async contexts
- Use `tokio::spawn` for creating concurrent tasks

**Memory Management**: 
- Prefer borrowing over ownership
- **Use `dashmap` for concurrent hashmaps** instead of `Arc<RwLock<HashMap<_, _>>>` (dashmap provides atomic operations and is more efficient)
- Avoid unnecessary `clone()`, consider `Cow<T>` or reference counting

**Testing**: 
- Unit tests alongside implementation code
- Integration tests where appropriate
- Use `assert_eq!`, `assert_ne!` for assertions
- Run specific tests: `cargo test <test_name> -p <crate>`

**Documentation**:
- All public APIs must have documentation comments (`///`)
- Include usage examples in complex API documentation
- Keep README.md and other docs updated

## Cursor Rules Summary

**Development Standards**:
- Line length: 100 characters
- 4-space indentation (no tabs)
- Documentation comments for all public APIs
- Use `cargo fmt` and `cargo clippy` before commits

**Error Handling**:
- `anyhow` for application code, `thiserror` for libraries
- Contextual error messages with `anyhow::Context`
- No sensitive data in error messages

**Module Organization**:
- Clear module responsibilities
- `pub(crate)` for internal visibility
- Re-export public APIs in `lib.rs`

**Dependencies**:
- Centralized workspace dependency management
- Specific versions (no `*`)
- Regular security audits with `cargo audit`

## Build System

The project uses a sophisticated Makefile with Docker buildx for cross-platform compilation:

### Docker Build Commands
```bash
# Check Docker buildx availability
make check-buildx

# Setup buildx builder (if needed)
make setup-buildx

# Build document-parser for specific platforms
make build-document-parser-x86_64
make build-document-parser-arm64
make build-document-parser-multi

# Build voice-cli for specific platforms
make build-voice-cli-x86_64
make build-voice-cli-arm64
make build-voice-cli-multi

# Build all components
make build-all-x86_64
make build-all-arm64
make build-all-multi

# Build and run Docker runtime image
make build-image
make run
```

**Build System Features**:
- **Docker-based builds**: All compilation happens in containers for consistency
- **Multi-platform support**: Linux x86_64 and ARM64 targets
- **Export targets**: Separate build and runtime stages
- **Automated dependency installation**: Python and Rust dependencies managed in containers
- **Output directory**: `./dist/` contains all built binaries organized by platform

## Service-Specific Architecture Details

### Document Parser (`document-parser/`)
- **Core Structure**: `app_state.rs`, `config.rs`, `main.rs`, `lib.rs`
- **Submodules**: `handlers/`, `middleware/`, `models/`, `parsers/`, `processors/`, `services/`, `tests/`, `utils/`
- **Python Integration**: MinerU for PDF parsing, MarkItDown for other formats
- **Virtual Environment**: Auto-managed in `./venv/`, activated via `source ./venv/bin/activate`
- **Server**: Axum-based HTTP server with multipart file upload support
- **Configuration**: YAML/JSON/TOML support with environment variable overrides

### Voice CLI (`voice-cli/`)
- **Core Components**:
  - `services/`: Model management, transcription engine, TTS service, task queue
  - `server/`: HTTP handlers, routes, middleware configuration
  - `models/`: Request/response data structures
- **Whisper Integration**: Model download and management via `voice-toolkit`
- **TTS Service**: Python-based with `uv` dependency management
- **FFmpeg**: Metadata extraction via `ffmpeg-sidecar`
- **Apalis**: Async task processing with SQLite persistence

### MCP Proxy (`mcp-proxy/`)
- **Core Structure**: `config.rs`, `lib.rs`, `main.rs`, `mcp_error.rs`
- **Submodules**: `client/`, `model/`, `proxy/`, `server/`, `tests/`
- **SSE Protocol**: Real-time communication via Server-Sent Events
- **Plugin System**: Dynamic MCP service loading and management
- **HTTP API**: REST endpoints for service management and status checks

## Common Patterns

**Service Initialization**: All services follow similar patterns for configuration loading, logging setup, and graceful shutdown.

**HTTP API Design**: Consistent use of axum extractors, middleware configuration, and OpenAPI documentation.

**Async Task Processing**: Voice services use apalis for background task processing with retry mechanisms and SQLite persistence.

**Python Integration**: Both document-parser and voice-cli use Python services with uv for dependency management and virtual environment handling.

**Configuration Management**: Hierarchical configuration with environment variable overrides and command-line argument integration.

## Single Test Execution Examples
```bash
# Run tests for specific crate
cargo test -p mcp-proxy
cargo test -p voice-cli
cargo test -p document-parser

# Run specific test
cargo test test_extract_basic_metadata -p voice-cli
cargo test <test_name> -p mcp-proxy

# Run tests in release mode
cargo test --release -p mcp-proxy

# Run library tests only (excluding integration tests)
cargo test --lib -p voice-cli

# Run tests with output
cargo test -p mcp-proxy -- --nocapture
```

## Python/uv Environment Management

### For Document Parser:
```bash
cd document-parser
# Initialize Python environment (creates ./venv/)
cargo run --bin document-parser -- uv-init

# Check environment status
cargo run --bin document-parser -- check

# Start server
cargo run --bin document-parser -- server

# Troubleshoot issues
cargo run --bin document-parser -- troubleshoot
```

### For Voice CLI TTS:
```bash
cd voice-cli
# Install uv package manager
curl -LsSf https://astral.sh/uv/install.sh | sh

# Install Python dependencies
uv sync

# Run TTS service directly
python3 tts_service.py --help
```

## Dependencies Management

All dependencies are managed centrally in the workspace `Cargo.toml`:
- Sub-crates use `{ workspace = true }` for dependency references
- Specific versions (no `*` wildcards)
- Centralized feature flags
- Regular security audits with `cargo audit`

Key workspace dependencies:
- `rmcp`: MCP protocol implementation with SSE support
- `tokio`: Async runtime
- `axum`: Web framework with tower middleware
- `tracing`: Structured logging
- `apalis`: Async task queue
- `dashmap`: Concurrent hashmap (preferred over `Arc<RwLock<HashMap>>`)