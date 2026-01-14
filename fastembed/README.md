# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

Text embedding HTTP service using FastEmbed library for efficient text vectorization.

## Overview

`fastembed` is a high-performance text embedding service built with Rust, providing HTTP API for text vectorization using FastEmbed.

## Features

- **FastEmbed Integration**: Uses FastEmbed 5.0 with ONNX runtime
- **HTTP API**: RESTful API for text embedding
- **Concurrent Processing**: DashMap for efficient concurrent operations
- **OpenAPI Documentation**: Auto-generated API docs
- **Multiple Models**: Support for various embedding models

## Quick Start

### Installation

```bash
# Build from source
cargo build --release -p fastembed

# Binary location
ls target/release/fastembed
```

### Usage

```bash
# Start server (default port 8080)
fastembed server

# Specify custom port
fastembed server --port 8081
```

### API Usage

```bash
# Generate embeddings
curl -X POST http://localhost:8080/embed \
  -H "Content-Type: application/json" \
  -d '{
    "texts": ["Hello world", "Fast embedding"],
    "model": "BAAI/bge-small-en-v1.5"
  }'
```

## Supported Models

- `BAAI/bge-small-en-v1.5` - Fast English model (384 dimensions)
- `BAAI/bge-base-en-v1.5` - Balanced English model (768 dimensions)
- `BAAI/bge-large-en-v1.5` - High-quality English model (1024 dimensions)

## Development

```bash
# Build
cargo build -p fastembed

# Test
cargo test -p fastembed
```

## License

MIT OR Apache-2.0

## Contributing

Issues and Pull Requests are welcome!
