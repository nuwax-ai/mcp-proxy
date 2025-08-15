# Project Structure

## Workspace Organization
This is a Cargo workspace with two main crates:
- `document-parser/`: Document processing service
- `mcp-proxy/`: MCP proxy service

## Root Level Files
- `Cargo.toml`: Workspace configuration with shared dependencies
- `README.md`: Project documentation (Chinese)
- `CHANGELOG.md`: Version history
- `cliff.toml`: Changelog generation config
- `deny.toml`: Security audit configuration
- `_typos.toml`: Spell check configuration
- `.pre-commit-config.yaml`: Code quality hooks

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

## MCP Proxy Structure
```
mcp-proxy/
├── src/
│   ├── main.rs              # Application entry point
│   ├── lib.rs               # Library exports
│   ├── config.rs            # Configuration management
│   ├── mcp_error.rs         # Error handling
│   ├── client/              # MCP client implementation
│   │   ├── mod.rs
│   │   └── sse_client.rs          # SSE client
│   ├── proxy/               # Proxy logic
│   │   ├── mod.rs
│   │   └── proxy_handler.rs       # Request proxying
│   ├── server/              # HTTP server
│   │   ├── mod.rs
│   │   ├── router_layer.rs         # Route management
│   │   ├── mcp_dynamic_router_service.rs # Dynamic routing
│   │   ├── handlers/               # Request handlers
│   │   │   ├── mod.rs
│   │   │   ├── health.rs           # Health endpoints
│   │   │   ├── mcp_add_handler.rs  # MCP service registration
│   │   │   ├── mcp_check_status_handler.rs # Status checking
│   │   │   ├── run_code_handler.rs # Code execution
│   │   │   ├── delete_route_handler.rs # Route removal
│   │   │   ├── check_mcp_is_status.rs # Service validation
│   │   │   └── sse_server.rs       # SSE endpoints
│   │   ├── middlewares/            # HTTP middleware
│   │   │   ├── mod.rs
│   │   │   ├── auth.rs             # Authentication
│   │   │   ├── request_logger.rs   # Request logging
│   │   │   ├── request_id.rs       # Request tracking
│   │   │   ├── server_time.rs      # Timing middleware
│   │   │   ├── mark_log_span.rs    # Tracing spans
│   │   │   ├── mcp_router_json.rs  # JSON handling
│   │   │   └── mcp_update_latest_layer.rs # State updates
│   │   └── task/                   # Background tasks
│   │       ├── mod.rs
│   │       ├── mcp_start_task.rs   # Service startup
│   │       ├── schedule_task.rs    # Task scheduling
│   │       └── schedule_check_mcp_live.rs # Health monitoring
│   ├── model/               # Data models
│   │   ├── mod.rs
│   │   ├── app_state_model.rs      # Application state
│   │   ├── global.rs               # Global state management
│   │   ├── http_result.rs          # HTTP responses
│   │   ├── mcp_check_status_model.rs # Status checking
│   │   ├── mcp_config.rs           # MCP configuration
│   │   └── mcp_router_model.rs     # Routing models
│   └── tests/               # Unit tests
├── benches/                 # Performance benchmarks
├── examples/                # Usage examples
├── fixtures/                # Test files
├── logs/                    # Log output directory
└── config.yml              # Default configuration
```

## Configuration Files
- Each service has its own `config.yml` with service-specific settings
- Environment variables override config file values
- Logging configuration supports both console and file output
- OSS and external service credentials via environment variables

## Code Organization Patterns
- **Handlers**: HTTP request/response logic only
- **Services**: Business logic and orchestration
- **Models**: Data structures and serialization
- **Utils**: Pure functions and utilities
- **Tests**: Co-located with source code, integration tests separate
- **Benchmarks**: Performance testing for critical paths

## Naming Conventions
- **Files**: snake_case (e.g., `document_handler.rs`)
- **Modules**: snake_case matching file names
- **Structs/Enums**: PascalCase (e.g., `DocumentTask`)
- **Functions/Variables**: snake_case (e.g., `parse_document`)
- **Constants**: SCREAMING_SNAKE_CASE (e.g., `APP_VERSION`)

## Import Organization
1. Standard library imports
2. External crate imports (workspace dependencies first)
3. Local crate imports (relative modules)
4. Re-exports in mod.rs files for clean public APIs