---
inclusion: always
---

# Project Structure & Organization

## Workspace Architecture

This is a Cargo workspace with four main crates:
- `document-parser/`: Multi-format document processing service
- `mcp-proxy/`: MCP protocol proxy with SSE support  
- `voice-cli/`: Audio transcription service with Apalis task queue
- `oss-client/`: Shared OSS storage client library

**Key Principle**: No root-level implementation code - all functionality must be in workspace members.

## Workspace Dependencies

**Shared Dependencies**: All common dependencies are defined in workspace `Cargo.toml` and referenced with `{ workspace = true }` in member crates.

**Key Shared Crates**:
- `axum`, `tokio`: HTTP framework and async runtime
- `serde`, `serde_json`: Serialization
- `anyhow`, `thiserror`: Error handling
- `tracing`: Structured logging
- `uuid`, `chrono`: Utilities

## Document Parser Structure

```
document-parser/
├── src/
│   ├── main.rs              # Application entry point
│   ├── lib.rs               # Library exports and constants
│   ├── config.rs            # Configuration management
│   ├── error.rs             # Error types and handling
│   ├── app_state.rs         # Shared application state
│   ├── routes.rs            # HTTP route definitions
│   ├── handlers/            # HTTP request handlers
│   │   ├── mod.rs
│   │   ├── document_handler.rs    # Document upload/processing
│   │   ├── task_handler.rs        # Task status management
│   │   ├── health_handler.rs      # Health checks
│   │   ├── toc_handler.rs         # Table of contents
│   │   └── markdown_handler.rs    # Markdown processing
│   ├── models/              # Data structures
│   │   ├── mod.rs
│   │   ├── document_task.rs       # Task representation
│   │   ├── document_format.rs     # File format enum
│   │   ├── parser_engine.rs       # Engine selection
│   │   ├── task_status.rs         # Status tracking
│   │   ├── structured_document.rs # Parsed document structure
│   │   ├── oss_data.rs           # Cloud storage data
│   │   ├── http_result.rs        # API response wrapper
│   │   ├── parse_result.rs       # Parsing results
│   │   └── toc_item.rs           # Table of contents items
│   ├── parsers/             # Document parsing engines
│   │   ├── mod.rs
│   │   ├── parser_trait.rs        # Common parser interface
│   │   ├── mineru_parser.rs       # PDF parsing (MinerU)
│   │   ├── markitdown_parser.rs   # Multi-format parsing
│   │   ├── dual_engine_parser.rs  # Engine coordination
│   │   └── format_detector.rs     # Format detection
│   ├── processors/          # Content processing
│   │   ├── mod.rs
│   │   └── markdown_processor.rs  # Markdown manipulation
│   ├── services/            # Business logic
│   │   ├── mod.rs
│   │   ├── document_service.rs    # Document processing orchestration
│   │   ├── task_service.rs        # Task management
│   │   ├── task_queue_service.rs  # Async task queue
│   │   ├── storage_service.rs     # Local storage
│   │   ├── oss_service.rs         # Cloud storage
│   │   └── image_processor.rs     # Image handling
│   ├── utils/               # Utility functions
│   │   ├── mod.rs
│   │   ├── file_utils.rs          # File operations
│   │   ├── format_utils.rs        # Format detection
│   │   ├── logging.rs             # Logging setup
│   │   ├── health_check.rs        # Health monitoring
│   │   ├── metrics.rs             # Performance metrics
│   │   ├── alerting.rs            # Error alerting
│   │   └── environment_manager.rs # Python env management
│   └── tests/               # Unit tests
├── tests/                   # Integration tests
├── benches/                 # Performance benchmarks
├── fixtures/                # Test data
└── config.yml              # Default configuration
```

## Voice CLI Structure (Apalis-based)

```
voice-cli/
├── src/
│   ├── main.rs              # Application entry point
│   ├── lib.rs               # Library exports
│   ├── config.rs            # Configuration management
│   ├── error.rs             # Error types
│   ├── cli/                 # CLI interface
│   │   ├── mod.rs
│   │   ├── model.rs               # CLI model commands
│   │   ├── server.rs              # Server commands
│   │   └── unified_handlers.rs    # Command handlers
│   ├── daemon/              # Background service
│   │   ├── mod.rs
│   │   ├── background_service.rs  # Service abstraction
│   │   ├── service_logging.rs     # Daemon logging
│   │   └── services/              # Service implementations
│   ├── models/              # Data structures
│   │   ├── mod.rs
│   │   ├── config.rs              # Configuration models
│   │   ├── task.rs                # Task representation
│   │   ├── stepped_task.rs        # Multi-step task workflow
│   │   ├── worker.rs              # Worker configuration
│   │   ├── request.rs             # API request models
│   │   └── http_result.rs         # Response wrapper
│   ├── server/              # HTTP server
│   │   ├── mod.rs
│   │   ├── routes.rs              # Route definitions
│   │   ├── handlers.rs            # Request handlers
│   │   ├── middleware.rs          # HTTP middleware
│   │   └── http_tracing.rs        # Request tracing
│   ├── services/            # Business logic
│   │   ├── mod.rs
│   │   ├── apalis_sqlite.rs       # Apalis SQLite backend
│   │   ├── apalis_transcription.rs # Transcription worker
│   │   ├── stepped_worker.rs      # Multi-step task worker
│   │   ├── transcription_engine.rs # Core transcription logic
│   │   ├── transcription_steps.rs # Step implementations
│   │   ├── audio_file_manager.rs  # Audio file handling
│   │   ├── audio_format_detector.rs # Format detection
│   │   ├── model_service.rs       # Model management
│   │   ├── task_store.rs          # Task persistence
│   │   ├── task_recovery.rs       # Task recovery logic
│   │   └── worker_pool.rs         # Worker management
│   ├── utils/               # Utilities
│   │   ├── mod.rs
│   │   ├── cleanup.rs             # Resource cleanup
│   │   └── signal_handling.rs     # Graceful shutdown
│   └── tests/               # Unit tests
├── tests/                   # Integration tests
├── templates/               # Configuration templates
└── logs/                    # Log output
```

## Module Organization Patterns

### Layer Responsibilities
- **Handlers**: HTTP request/response logic only - no business logic
- **Services**: Business logic and orchestration - core functionality
- **Models**: Data structures with serde serialization - shared types
- **Utils**: Pure functions and utilities - no state
- **CLI**: Command-line interface - user interaction
- **Daemon**: Background services - long-running processes

### File Placement Rules
- **Unit tests**: `#[cfg(test)]` modules in same file as implementation
- **Integration tests**: Separate `tests/` directory
- **Benchmarks**: `benches/` directory for performance testing
- **Examples**: `examples/` directory for usage demonstrations
- **Fixtures**: Test data and configuration samples

### State Management
- **Shared state**: Use `Arc<DashMap<K, V>>` for concurrent access
- **Application state**: Centralized in `app_state.rs` or similar
- **Configuration**: YAML files with environment variable overrides
- **Persistence**: Sled for key-value, SQLite for structured data

## Code Organization Rules

### Naming Conventions
- **Files/Modules**: `snake_case` (e.g., `audio_file_manager.rs`)
- **Structs/Enums**: `PascalCase` (e.g., `TranscriptionTask`)
- **Functions/Variables**: `snake_case` (e.g., `process_audio_file`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `MAX_FILE_SIZE`)
- **Traits**: `PascalCase` with descriptive names (e.g., `AudioProcessor`)

### Import Organization (Required Order)
```rust
// 1. Standard library
use std::collections::HashMap;
use std::path::PathBuf;

// 2. External crates (workspace deps first)
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::fs;

// 3. Local crate imports
use crate::models::Task;
use crate::services::AudioService;

// 4. Re-exports in mod.rs only
pub use self::handler::*;
```

### Module Visibility
- Use `pub(crate)` for internal APIs
- Public exports only in `lib.rs`
- Avoid `pub` unless truly needed externally
- Document all public APIs with `///` comments

### Error Handling Patterns
- **Services**: Return `Result<T, anyhow::Error>` with context
- **Handlers**: Convert to HTTP responses with proper status codes
- **Models**: Use `thiserror` for structured error types
- **Never**: Use `unwrap()` or `panic!()` in production code
