# Technology Stack

## Build System
- **Language**: Rust (edition 2024)
- **Build Tool**: Cargo with workspace configuration
- **Target**: Single binary deployment with zero runtime dependencies

## Core Dependencies

### HTTP Framework & Async Runtime
- **Axum**: High-performance HTTP framework with type safety
- **Tokio**: Async runtime with full feature set (macros, net, rt-multi-thread, signal, io-util)
- **Tower/Tower-HTTP**: Middleware and service abstractions with compression, CORS, tracing

### Serialization & Configuration
- **Serde**: JSON/YAML serialization with derive features
- **Clap**: CLI argument parsing with derive and env features
- **YAML**: Configuration file format (serde_yaml)

### Storage & External Services
- **Sled**: Embedded key-value database for task storage
- **AWS SDK**: S3 integration for OSS storage (aws-config, aws-sdk-s3)
- **Reqwest**: HTTP client with streaming and JSON support

### Document Processing
- **pulldown-cmark**: Markdown parsing and processing
- **pulldown-cmark-toc**: Table of contents generation
- **MinerU**: PDF parsing engine (Python integration)
- **MarkItDown**: Multi-format document parsing (Python integration)

### MCP Integration
- **rmcp**: MCP protocol implementation with multiple transport layers
- **run_code_rmcp**: Code execution capabilities for JS/TS/Python

### Utilities
- **UUID**: Unique identifier generation (v4, v7)
- **Chrono**: Date/time handling with serde support
- **Anyhow/Thiserror**: Error handling
- **Tracing**: Structured logging with file appender
- **DashMap**: Concurrent HashMap for shared state

## Common Commands

### Development
```bash
# Build in development mode
cargo build

# Run with hot reload (if using cargo-watch)
cargo watch -x run

# Run tests
cargo test
cargo nextest run --all-features

# Run benchmarks
cargo bench

# Format code
cargo fmt

# Lint code
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
```

### Production Build
```bash
# Optimized release build
RUSTFLAGS="-C target-cpu=native" cargo build --release

# Cross-platform build (Linux musl)
cargo build --release --target x86_64-unknown-linux-musl
```

### Quality Assurance
```bash
# Run pre-commit hooks
pre-commit run --all-files

# Security audit
cargo deny check -d

# Spell check
typos

# Generate changelog
git cliff
```

### Environment Setup
```bash
# Install required tools
cargo install cargo-nextest cargo-deny typos-cli git-cliff cargo-generate

# Install pre-commit
pipx install pre-commit
pre-commit install
```

## Architecture Patterns
- **Workspace**: Multi-crate workspace with shared dependencies
- **Async/Await**: Tokio-based async programming throughout
- **Error Handling**: Anyhow for application errors, thiserror for library errors
- **Configuration**: YAML-based config with environment variable overrides
- **Logging**: Structured logging with tracing, both console and file output
- **State Management**: Shared application state using Arc and async-safe collections

## Environment Variables
Key environment variables for configuration:
- `RUST_LOG`: Logging level control
- `ALIYUN_OSS_*`: OSS storage credentials
- `MINERU_*`: MinerU engine configuration
- `MARKITDOWN_*`: MarkItDown engine configuration