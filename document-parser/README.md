# Document Parser

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# Document Parser

A high-performance multi-format document parsing service supporting PDF, Word, Excel, and PowerPoint with GPU acceleration capabilities.

## Features

- 🚀 **High-Performance Parsing**: MinerU and MarkItDown dual-engine support
- 🎯 **GPU Acceleration**: CUDA/sglang support for GPU acceleration (optional)
- 🔧 **Zero-Configuration Deployment**: Automatic environment detection and dependency installation
- 📚 **Multi-Format Support**: PDF, Word, Excel, PowerPoint, Markdown, and more
- 🌐 **HTTP API**: RESTful API interface for easy integration
- 📊 **Real-time Monitoring**: Built-in performance monitoring and health checks
- ☁️ **OSS Integration**: Alibaba Cloud OSS support for cloud storage

## Quick Start

### 1. Environment Initialization

```bash
cd document-parser

# Initialize uv virtual environment and dependencies (first time)
document-parser uv-init

# Check environment status
document-parser check
```

### 2. Start Service

```bash
# Start document parsing service
document-parser server

# Or specify custom port
document-parser server --port 8088
```

The service will start at `http://localhost:8087` (default) and automatically activate the virtual environment.

## System Requirements

### Basic Requirements
- **Rust**: 1.70+
- **Python**: 3.8+
- **uv**: Python package manager

### GPU Acceleration (Optional)
- **NVIDIA GPU**: CUDA-compatible
- **CUDA Toolkit**: 11.8+
- **GPU Memory**: At least 8GB recommended

## Supported Formats

| Format | Parsing Engine | Features |
|--------|----------------|----------|
| PDF | MinerU | Professional PDF parsing, image extraction, table recognition |
| Word | MarkItDown | Document structure preservation, format conversion |
| Excel | MarkItDown | Table data extraction, format preservation |
| PowerPoint | MarkItDown | Slide content extraction, image saving |
| Markdown | Built-in | Real-time parsing, table of contents generation |

## Configuration

### Basic Configuration

```yaml
# Server configuration
server:
  port: 8087
  host: "0.0.0.0"

# MinerU configuration
mineru:
  backend: "vlm-sglang-engine"  # Enable GPU acceleration
  max_concurrent: 3
  quality_level: "Balanced"
```

### GPU Acceleration Configuration

```yaml
mineru:
  backend: "vlm-sglang-engine"  # Use sglang backend
  max_concurrent: 2              # Lower concurrency for GPU
  batch_size: 1
```

## Common Commands

```bash
# Environment management
document-parser check              # Check environment status
document-parser uv-init            # Initialize environment
document-parser troubleshoot       # Troubleshooting guide

# Service management
document-parser server             # Start service
document-parser server --port 8088 # Specify port

# File parsing (CLI)
document-parser parse --input file.pdf --output result.md --parser mineru
```

## API Usage

### Parse Document

```bash
curl -X POST "http://localhost:8087/api/v1/documents/parse" \
  -H "Content-Type: multipart/form-data" \
  -F "file=@document.pdf" \
  -F "format=pdf"
```

### Get Parsing Status

```bash
curl "http://localhost:8087/api/v1/documents/{task_id}/status"
```

### API Documentation

Once the service is running, visit:
- **OpenAPI Swagger UI**: `http://localhost:8087/swagger-ui/`
- **OpenAPI JSON**: `http://localhost:8087/api-docs/openapi.json`

## Performance Optimization

### GPU Acceleration

1. Ensure `sglang[all]` is installed
2. Configure `backend: "vlm-sglang-engine"`
3. Adjust concurrency parameters based on GPU memory
4. Monitor GPU usage

### Concurrency Control

```yaml
mineru:
  max_concurrent: 2    # Adjust based on system performance
  batch_size: 1        # Process in small batches
  queue_size: 100      # Queue buffer size
```

## Troubleshooting

### Common Issues

1. **Virtual environment not activated**: Run `source ./venv/bin/activate`
2. **Dependency installation failed**: Run `document-parser uv-init`
3. **GPU acceleration not working**: Refer to CUDA Environment Setup Guide
4. **Permission issues**: Check directory and user permissions

### Get Help

```bash
# Detailed troubleshooting guide
document-parser troubleshoot

# Environment status check
document-parser check

# View logs
tail -f logs/log.$(date +%Y-%m-%d)
```

## Development

### Build

```bash
cargo build --release
```

### Test

```bash
cargo test
```

### Code Check

```bash
cargo fmt
cargo clippy
```

## License

This project is licensed under MIT License.

## Contributing

Issues and Pull Requests are welcome!
