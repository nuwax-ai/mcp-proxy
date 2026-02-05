# MCP-Proxy Workspace

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# MCP-Proxy Workspace

A comprehensive Rust workspace implementing MCP (Model Context Protocol) proxy system with multiple services including document parsing, voice transcription, and protocol conversion.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org/)

## Workspace Members

| Crate | Version | Description |
|-------|---------|-------------|
| **mcp-common** | 0.1.5 | Shared types and utilities for MCP proxy components |
| **mcp-sse-proxy** | 0.1.5 | SSE (Server-Sent Events) proxy implementation using rmcp 0.10 |
| **mcp-streamable-proxy** | 0.1.5 | Streamable HTTP proxy implementation using rmcp 0.12 |
| **mcp-stdio-proxy** | 0.1.18 | Main MCP proxy server with CLI tool for protocol conversion |
| **document-parser** | 0.1.0 | High-performance multi-format document parsing service |
| **voice-cli** | 0.1.0 | Speech-to-text HTTP service with Whisper model support |
| **oss-client** | 0.1.0 | Lightweight Alibaba Cloud OSS client library |
| **fastembed** | 0.1.0 | Text embedding HTTP service using FastEmbed |

## Quick Start

### Prerequisites

- **Rust**: 1.70 or later (recommended 1.75+)
- **Python**: 3.8+ (for document-parser and voice-cli TTS)
- **uv**: Python package manager (install via `curl -LsSf https://astral.sh/uv/install.sh | sh`)

### Installation

#### Method 1: Pre-built Binaries (Recommended)

**Using installation script (Linux/macOS):**
```bash
# Install mcp-proxy
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.sh | sh

# Install document-parser
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.sh | sh

# Install voice-cli
curl --proto '=https' --tlsv1.2 -sSf https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/voice-cli-installer.sh | sh
```

**Using installation script (Windows PowerShell):**
```powershell
# Install mcp-proxy
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/mcp-stdio-proxy-installer.ps1 | iex

# Install document-parser
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/document-parser-installer.ps1 | iex

# Install voice-cli
irm https://github.com/nuwax-ai/mcp-proxy/releases/latest/download/voice-cli-installer.ps1 | iex
```

**Download from GitHub Releases:**
Visit [GitHub Releases](https://github.com/nuwax-ai/mcp-proxy/releases) to download binaries for your platform.

Supported platforms:
- Linux x86_64
- Linux ARM64
- macOS Intel (x86_64)
- macOS Apple Silicon (ARM64)
- Windows x86_64

#### Method 2: cargo install

```bash
cargo install mcp-stdio-proxy
```

#### Method 3: Build from Source

```bash
# Clone repository
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy

# Build all workspace members
cargo build --release

# Or build specific crates
cargo build -p mcp-proxy
cargo build -p document-parser
cargo build -p voice-cli
```

### MCP Proxy (mcp-stdio-proxy)

The main proxy service that converts SSE/Streamable HTTP to stdio protocol.

```bash
# Install from source
cargo install --path ./mcp-proxy

# Start the proxy server
mcp-proxy

# Convert remote MCP service to stdio
mcp-proxy convert https://example.com/mcp/sse

# Check service status
mcp-proxy check https://example.com/mcp/sse

# Detect protocol type
mcp-proxy detect https://example.com/mcp
```

**See:** [mcp-proxy/README.md](./mcp-proxy/README.md) for detailed documentation.

### Document Parser

High-performance document parsing service supporting PDF, Word, Excel, and PowerPoint.

```bash
cd document-parser

# Initialize Python environment (first time)
document-parser uv-init

# Check environment status
document-parser check

# Start HTTP server
document-parser server
```

**See:** [document-parser/README.md](./document-parser/README.md) for detailed documentation.

### Voice CLI

Speech-to-text HTTP service with Whisper model support.

```bash
cd voice-cli

# Initialize server configuration
voice-cli server init

# Run voice server
voice-cli server run

# List Whisper models
voice-cli model list

# Download model
voice-cli model download tiny
```

**See:** [voice-cli/README.md](./voice-cli/README.md) for detailed documentation.

## Architecture

### Core Services

#### 1. MCP Proxy System

- **mcp-common**: Shared configuration types and utilities
- **mcp-sse-proxy**: SSE protocol support (rmcp 0.10)
- **mcp-streamable-proxy**: Streamable HTTP protocol support (rmcp 0.12)
- **mcp-stdio-proxy**: Main CLI tool for protocol conversion

**Features:**
- Multi-protocol support: SSE, Streamable HTTP, stdio
- Dynamic plugin loading
- Protocol auto-detection and conversion
- OpenTelemetry integration with OTLP
- Background health checks

#### 2. Document Parser

**Features:**
- Multi-format support: PDF (MinerU), Word/Excel/PowerPoint (MarkItDown)
- GPU acceleration via CUDA/sglang (optional)
- Python environment management with uv
- HTTP API with OpenAPI documentation
- OSS integration for cloud storage

#### 3. Voice CLI

**Features:**
- Whisper model integration (tiny/base/small/medium/large)
- Multi-format audio support (MP3, WAV, FLAC, M4A, etc.)
- Apalis-based async task queue with SQLite persistence
- FFmpeg integration for metadata extraction
- **TTS service (TODO - currently has issues)**

#### 4. Utility Libraries

- **oss-client**: Alibaba Cloud OSS client with unified interface
- **fastembed**: Text embedding HTTP service using FastEmbed

## Development

### Build Commands

```bash
# Build all workspace crates
cargo build

# Build specific crate
cargo build -p mcp-proxy

# Build in release mode
cargo build --release

# Run tests for all crates
cargo test

# Run tests for specific crate
cargo test -p mcp-proxy

# Run clippy for linting
cargo clippy --all-targets --all-features

# Format code
cargo fmt
```

### Cross-Platform Building (Docker)

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

### Code Style

- Line length: 100 characters
- 4-space indentation (no tabs)
- Use `dashmap` for concurrent hashmaps instead of `Arc<RwLock<HashMap>>`
- Follow KISS and SOLID principles
- "Fail fast" error handling with `anyhow::Context`

## Documentation

- [CLAUDE.md](./CLAUDE.md) - Development guide for contributors
- [mcp-proxy/README.md](./mcp-proxy/README.md) - MCP Proxy documentation
- [document-parser/README.md](./document-parser/README.md) - Document Parser documentation
- [voice-cli/README.md](./voice-cli/README.md) - Voice CLI documentation
- [oss-client/README.md](./oss-client/README.md) - OSS Client documentation

## License

This project is dual-licensed under MIT OR Apache-2.0.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

- **GitHub Repository**: https://github.com/nuwax-ai/mcp-proxy
- **Issue Tracker**: https://github.com/nuwax-ai/mcp-proxy/issues
- **Discussions**: https://github.com/nuwax-ai/mcp-proxy/discussions

## Related Resources

- [MCP Official Documentation](https://modelcontextprotocol.io/)
- [rmcp - Rust MCP Implementation](https://crates.io/crates/rmcp)
- [MCP Servers List](https://github.com/modelcontextprotocol/servers)
