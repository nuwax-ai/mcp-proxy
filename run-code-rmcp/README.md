# MCP Code Execution Tool

A Rust-based tool that provides both a command-line interface and an MCP (Model Context Protocol) server for executing JavaScript, TypeScript, and Python code in isolated environments.

**Repository:** <https://github.com/nuwax-ai/run_code_rmcp>

English | [中文](./README_zh.md)

## Features

- Execute JavaScript, TypeScript, and Python code in isolated environments
- **JavaScript/TypeScript**: Powered by Deno runtime
- **Python**: Powered by `uv` with automatic dependency management
- Automatic code caching using blake3 hashing for faster repeated executions
- Support for ESM and CommonJS modules in JavaScript
- Automatic Python dependency parsing and installation
- Separate log capture and execution result handling
- Available as both CLI tool (`run_code_rmcp`) and MCP server (`script_runner`)

## Installation

### Install CLI Tool

```bash
# Clone the repository
git clone https://github.com/nuwax-ai/run_code_rmcp.git
cd run_code_rmcp

# Install the CLI tool (run_code_rmcp)
cargo install --path .

# Or install both CLI tool and MCP server
cargo install --path . --features mcp
cargo install --path . --bin script_runner --features mcp
```

### System Requirements

- **Rust**: 1.85 or higher (for building)
- **Deno**: Required for JavaScript/TypeScript execution
- **uv**: Required for Python execution and dependency management

Install Deno:
```bash
curl -fsSL https://deno.land/install.sh | sh
```

Install uv:
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

## Usage

### CLI Tool: run_code_rmcp

The `run_code_rmcp` command allows you to execute code directly from the command line.

#### Basic Syntax

```bash
run_code_rmcp [OPTIONS] <COMMAND>
```

**Options:**
- `--show-logs`: Display execution logs
- `--clear-cache`: Clear cache before execution
- `--use-mcp`: Use MCP SDK integration (requires mcp feature)

**Commands:**
- `js`: Execute JavaScript code
- `ts`: Execute TypeScript code
- `python`: Execute Python code
- `clear-cache`: Clear cached files

#### Execute JavaScript/TypeScript

```bash
# Execute a JavaScript file
run_code_rmcp --show-logs js -f fixtures/test_js.js

# Execute with parameters
run_code_rmcp --show-logs js -f fixtures/test_js_params.js -p '{"name":"User"}'

# Execute TypeScript
run_code_rmcp --show-logs ts -f fixtures/test_ts.ts

# Execute inline code
run_code_rmcp js -c "function handler() { return 'Hello!'; }"
```

#### Execute Python

```bash
# Execute a Python file
run_code_rmcp --show-logs python -f fixtures/test_python.py

# Execute with parameters
run_code_rmcp --show-logs python -f fixtures/test_python_params.py -p '{"a":10, "b":20}'

# Execute inline code
run_code_rmcp python -c "def handler(): return 'Hello from Python!'"
```

#### Cache Management

```bash
# Clear all cache
run_code_rmcp clear-cache --language all

# Clear specific language cache
run_code_rmcp clear-cache --language python
run_code_rmcp clear-cache --language js
run_code_rmcp clear-cache --language ts

# Clear cache before execution
run_code_rmcp --clear-cache js -f fixtures/test_js.js
```

### Using as a Rust Library

```toml
[dependencies]
run_code_rmcp = { git = "https://github.com/nuwax-ai/run_code_rmcp.git" }
```

```rust
use run_code_rmcp::{CodeExecutor, LanguageScript};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Execute JavaScript
    let js_result = CodeExecutor::execute_with_params_compat(
        "function handler() { return {success: true}; }",
        LanguageScript::Js,
        None
    ).await?;

    // Execute Python
    let py_result = CodeExecutor::execute_with_params_compat(
        "def handler(): return {'success': True}",
        LanguageScript::Python,
        None
    ).await?;

    Ok(())
}
```

### MCP Server: script_runner

The `script_runner` is an MCP server that provides tools for AI assistants.

#### Starting the Server

```bash
# Start MCP server
script_runner

# Start with verbose logging
script_runner --verbose
```

The server communicates via stdio and waits for MCP protocol JSON requests.

#### Available Tools

The MCP server provides the following tools:

1. **run_javascript** - Execute JavaScript code
2. **run_typescript** - Execute TypeScript code
3. **run_python** - Execute Python code

Each tool accepts:
- `code` (string): The code to execute
- `params` (object, optional): Parameters to pass to the handler function

#### Integration with AI Assistants

Add to your MCP client configuration (e.g., Claude Desktop):

```json
{
  "name": "script-runner",
  "command": "script_runner",
  "transport": "stdio"
}
```

## Code Structure

### Handler Functions

Your scripts should define a handler function that returns the result.

#### JavaScript/TypeScript

Supports both `main` and `handler` functions (main takes priority):

```javascript
// This function takes priority
function main(input) {
    console.log("Processing with main:", input);
    return { result: "Hello from main!" };
}

// This function is used if main is not defined
function handler(input) {
    console.log("Processing with handler:", input);
    return { result: "Hello from handler!" };
}
```

ESM modules are also supported:

```javascript
import { serve } from "https://deno.land/std/http/server.ts";

function handler(input) {
    return { message: "ESM works!" };
}
```

#### Python

Supports both `handler` and `main` functions (handler takes priority):

```python
import pandas as pd

# This function takes priority
def handler(args):
    print(f"Processing: {args}")
    data = pd.DataFrame({"a": [1, 2, 3]})
    return {"result": data.to_dict()}

# This function is used if handler is not defined
def main(args):
    return {"result": "Using main instead"}
```

### Parameter Passing

Parameters are passed to your handler function:

```bash
run_code_rmcp js -f script.js -p '{"name": "User", "count": 42}'
```

In your code:
```javascript
function handler(input) {
    // input = { name: "User", count: 42 }
    return `Hello ${input.name}, count: ${input.count}`;
}
```

### Python Dependencies

Python dependencies are automatically detected and installed:

```python
import requests
import pandas as pd

def handler(args):
    # Dependencies are automatically installed via uv
    response = requests.get("https://api.example.com")
    data = pd.DataFrame(response.json())
    return data.to_dict()
```

### Log Capture

All console output is captured separately from the return value:

```javascript
console.log("This goes to logs");
console.log("So does this");

function handler() {
    return "This is the result";
}
```

Result:
```json
{
  "logs": ["This goes to logs", "So does this"],
  "result": "This is the result",
  "error": null
}
```

## Development

```bash
# Build
cargo build

# Run with cargo
cargo run -- --show-logs js -f fixtures/test_js.js

# Run tests
cargo test

# Build with MCP feature
cargo build --features mcp

# Run MCP server with cargo
cargo run --bin script_runner --features mcp
```

## Environment Pre-warming

The project includes a warm-up function to cache common dependencies:

```rust
use run_code_rmcp::warm_up_all_envs;

#[tokio::main]
async fn main() -> Result<()> {
    // Pre-install common Python and JS/TS dependencies
    warm_up_all_envs(None, None, None, None).await?;
    Ok(())
}
```

## TODO

- [ ] TTS (Text-to-Speech) functionality - Currently has known issues

## License

This project is licensed under the Apache License 2.0. See `LICENSE` and `NOTICE` files in the root directory for details.
