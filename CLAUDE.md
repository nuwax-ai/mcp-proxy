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

**FFmpeg Integration**: Lightweight FFmpeg command execution via `ffmpeg-sidecar` for media metadata extraction.

### Configuration System

All services use hierarchical configuration:
1. Default values in code
2. Configuration files (YAML/JSON/TOML)
3. Environment variables with service prefixes
4. Command-line arguments

### Development Standards

**Code Organization**: Strict workspace structure - no code in root directory. All implementation in sub-crates with clear module boundaries.

**Error Handling**: Prefer `anyhow` for application code, `thiserror` for libraries. Avoid `unwrap()` except in tests. Use `?` operator for error propagation.

**Concurrency**: Use `tokio` for async, `Arc<Mutex<T>>` or `Arc<RwLock<T>>` for shared state. Avoid blocking operations in async contexts.

**Memory Management**: Prefer borrowing over ownership. Use `dashmap` for concurrent hashmaps instead of `Arc<RwLock<HashMap<_, _>>>`.

**Testing**: Unit tests alongside implementation code. Integration tests where appropriate. Use `assert_eq!`, `assert_ne!` for assertions.

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

- **Docker-based builds**: All compilation happens in containers for consistency
- **Multi-platform support**: Linux x86_64 and ARM64 targets
- **Export targets**: Separate build and runtime stages
- **Automated dependency installation**: Python and Rust dependencies managed in containers

## Common Patterns

**Service Initialization**: All services follow similar patterns for configuration loading, logging setup, and graceful shutdown.

**HTTP API Design**: Consistent use of axum extractors, middleware configuration, and OpenAPI documentation.

**Async Task Processing**: Voice services use apalis for background task processing with retry mechanisms and SQLite persistence.

**Python Integration**: Both document-parser and voice-cli use Python services with uv for dependency management and virtual environment handling.

**Configuration Management**: Hierarchical configuration with environment variable overrides and command-line argument integration.