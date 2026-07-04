# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a Rust-based MCP (Model Context Protocol) server that executes JavaScript, TypeScript, and Python code in isolated environments. The project provides both a command-line tool and a library for code execution with proper sandboxing and result capture.

## Key Commands

### Building and Running
```bash
# Build the project
cargo build

# Run with cargo
cargo run -- js -f fixtures/test_js.js
cargo run -- ts -f fixtures/test_ts.ts  
cargo run -- python -f fixtures/test_python.py

# Install as binary
cargo install --path . --bin script_runner

# Run tests
cargo test

# Clear cache
cargo run -- clear-cache all
```

### Common Development Commands
```bash
# Execute JavaScript with logs
cargo run -- --show-logs js -f fixtures/test_js.js

# Execute with parameters
cargo run -- --show-logs python -f fixtures/test_python_params.py -p '{"a":10, "b":20}'

# Execute inline code
cargo run -- js -c "function handler(input) { return 'Hello: ' + input.name; }" -p '{"name":"User"}'

# Use MCP integration
cargo run -- --use-mcp python -f fixtures/test_python.py
```

## Architecture

### Core Components
- **src/main.rs** - CLI entry point with command parsing using clap
- **src/lib.rs** - Library interface exporting public APIs
- **src/model/code_run_model.rs** - Core execution models and `CodeExecutor` struct
- **src/cache/** - Code caching system using blake3 hashing
- **src/mcp/** - MCP protocol server implementation

### Language Runners
- **src/deno_runner/js_runner.rs** - JavaScript execution via Deno
- **src/deno_runner/ts_runner.rs** - TypeScript execution via Deno  
- **src/python_runner/python_runner.rs** - Python execution via uv with dependency analysis
- **src/python_runner/dependencies/** - Python dependency parsing and management

### Execution Flow
1. Code is processed and cached using blake3 hashing
2. Language-specific runner prepares the execution environment
3. Code is wrapped with logging capture and handler function execution
4. Results are captured and returned as `CodeScriptExecutionResult`

## Important Patterns

### Handler Function Convention
Scripts must expose a `handler` function (JS/TS) or `handler`/`main` function (Python) that returns the final result:
```javascript
function handler(input) {
    return "Result: " + input.param;
}
```

### Parameter Passing
- JS/TS: Parameters passed directly to handler function
- Python: Parameters passed via `INPUT_JSON` environment variable

### Error Handling
- Uses `anyhow::Result` for error propagation
- Custom `app_error` module for application-specific errors
- Execution results include error information in `CodeScriptExecutionResult`

### Dependencies
- **Deno** for JavaScript/TypeScript execution
- **uv** for Python environment isolation  
- **rmcp** crate for MCP protocol implementation
- **tokio** for async runtime
- **serde** for JSON serialization

## Development Notes

### Testing
- Test fixtures in `fixtures/` directory for all supported languages
- Integration tests in `src/tests/`
- Use `--show-logs` flag during development to see execution output

### Caching
- Code processing results are cached based on content hash
- Use `--clear-cache` to invalidate cache when debugging
- Cache files stored in system temp directory

### MCP Integration
- Can run as standalone MCP server via `script_runner` binary
- Supports SSE and HTTP transports
- Tools: `run_javascript`, `run_typescript`, `run_python`