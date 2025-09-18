---
inclusion: always
---

# Rust Development Guidelines

## Project Architecture

### Workspace Structure
- **Multi-crate workspace**: `document-parser`, `mcp-proxy`, `oss-client`, `voice-cli`
- **No root-level code**: All implementation must be in workspace members
- **Shared dependencies**: Centralized in workspace `Cargo.toml` with `{ workspace = true }`
- **Single binary deployment**: Zero runtime dependencies target

### Module Organization
- **Clear separation of concerns**: handlers, services, models, utils
- **Handlers**: HTTP request/response logic only
- **Services**: Business logic and orchestration  
- **Models**: Data structures with serde serialization
- **Utils**: Pure functions and utilities
- Use `pub(crate)` for internal APIs, public exports in `lib.rs`

## Code Style & Quality

### Formatting & Linting
- `cargo fmt` for consistent formatting (100 char line limit)
- `cargo clippy --all-targets --all-features -- -D warnings` 
- Document all public APIs with `///` comments
- Use 4-space indentation, no tabs

### Naming Conventions
- `snake_case`: variables, functions, modules, files
- `PascalCase`: structs, enums, traits
- `SCREAMING_SNAKE_CASE`: constants
- Descriptive names, avoid abbreviations except common ones (`id`, `url`)

## Error Handling Strategy

### Library Selection
- **`anyhow`**: Application-layer error handling, error chaining
- **`thiserror`**: Library code, structured error types for public APIs
- Always use `Result<T, E>` over `panic!`
- Use `?` operator for error propagation
- Add context with `anyhow::Context`

### Best Practices
- Avoid `unwrap()` and `expect()` in library code
- Include debugging info in errors without exposing sensitive data
- Implement `Display` and `Error` traits for custom error types

## Async & Concurrency

### Tokio Patterns
- Use `async/await` syntax throughout
- `tokio::spawn` for concurrent tasks
- Avoid `std::sync` primitives in async contexts
- Use `Arc<Mutex<T>>` or `Arc<RwLock<T>>` for shared state

### Concurrent Data Structures
- **Use `DashMap` instead of `RwLock<HashMap>`** for concurrent maps
- `Arc<T>` for shared immutable data
- `tokio::sync` primitives for async coordination

## Performance & Memory

### Memory Management
- Prefer borrowing (`&T`) over ownership (`T`)
- Avoid unnecessary `clone()`, use `Cow<T>` or `Arc<T>`
- Pre-allocate container capacity in loops
- Use `Box<T>` for large stack data

### Caching & Storage
- `moka` for in-memory caching with TTL
- `sled` for embedded key-value persistence
- `sqlx` with SQLite for structured data

## HTTP & Web Services

### Axum Patterns
- Type-safe extractors and responses
- Custom error response types implementing `IntoResponse`
- `tower` middleware for cross-cutting concerns
- Route grouping for logical API organization

### API Design
- OpenAPI documentation with `utoipa`
- Structured logging for requests/responses
- Proper HTTP status codes
- Graceful shutdown handling

## Task Processing

### Apalis Integration
- Use `apalis` for background job processing
- SQLite backend for task persistence
- Implement stepped tasks for complex workflows
- Task recovery and retry mechanisms

### Audio Processing
- `symphonia` for audio format detection
- `voice-toolkit` for speech-to-text operations
- Proper cleanup of temporary audio files

## Testing & Quality Assurance

### Test Organization
- Unit tests with `#[cfg(test)]` modules
- Integration tests in `tests/` directory
- Descriptive test function names
- Use `assert_eq!`, `assert_ne!` macros

### Quality Tools
- `cargo nextest` for parallel test execution
- `cargo deny` for security auditing
- `typos` for spell checking
- `git-cliff` for changelog generation

## Security & Production

### Input Validation
- Validate all external inputs
- Use type system to prevent invalid states
- Sanitize data before logging
- Handle sensitive data appropriately

### Monitoring & Observability
- Structured logging with `tracing`
- OpenTelemetry integration for distributed tracing
- Health check endpoints
- Metrics collection for performance monitoring

## Project-Specific Rules

### Document Processing
- Dual-engine parsing: MinerU (PDF), MarkItDown (others)
- Automatic format detection
- OSS integration for file storage
- Real-time markdown processing with TOC generation

### MCP Proxy
- SSE protocol support for real-time communication
- Dynamic plugin configuration and loading
- Code execution sandboxing (JS/TS/Python)
- Service health monitoring

### Voice CLI
- Apalis-based task queue for transcription
- Audio format detection and validation
- Stepped task processing for complex workflows
- Proper daemon lifecycle management

## Development Workflow

### Commands
```bash
# Development
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings

# Quality checks
pre-commit run --all-files
cargo deny check -d
typos

# Production build
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

### Code Review Checklist
- [ ] Passes `cargo fmt` and `cargo clippy`
- [ ] Public APIs documented
- [ ] Proper error handling (no `unwrap()` abuse)
- [ ] Tests cover new functionality
- [ ] No performance regressions
- [ ] Security considerations addressed
- [ ] Logging appropriate and secure