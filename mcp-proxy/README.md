# mcp-stdio-proxy

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# mcp-stdio-proxy

MCP (Model Context Protocol) client proxy tool that converts remote MCP services (SSE/Streamable HTTP) to local stdio interface.

> **Package Name**: `mcp-stdio-proxy`
> **Command Name**: `mcp-proxy` (shorter)

## Core Features

`mcp-proxy` is a lightweight client proxy tool that solves one core problem:

**Enabling stdio-only MCP clients to access remote SSE/HTTP MCP services.**

### How It Works

```
Remote MCP Service (SSE/HTTP) ←→ mcp-proxy ←→ Local Application (stdio)
```

- **Input**: Remote MCP service URL (supports SSE or Streamable HTTP protocols)
- **Output**: Local stdio interface (standard input/output)
- **Purpose**: Protocol conversion + transparent proxy

## Features

- 🔄 **Protocol Conversion**: Auto-detect and convert SSE/Streamable HTTP → stdio
- 🌐 **Remote Access**: Enable local applications to access remote MCP services
- 🔍 **Auto Protocol Detection**: Intelligently identify service protocol types
- 🔐 **Authentication Support**: Custom Authorization header and HTTP headers
- ⚡ **Lightweight & Efficient**: No extra configuration needed, works out of the box

## Installation

### Install from crates.io (Recommended)

```bash
cargo install mcp-stdio-proxy
```

### Build from Source

```bash
git clone https://github.com/nuwax-ai/mcp-proxy.git
cd mcp-proxy/mcp-proxy
cargo build --release
# Binary located at: target/release/mcp-proxy
```

## Quick Start

### Basic Usage

```bash
# Convert remote SSE service to stdio
mcp-proxy convert https://example.com/mcp/sse

# Or use simplified syntax (backward compatible)
mcp-proxy https://example.com/mcp/sse
```

### Complete Example with Authentication

```bash
# Use Bearer token authentication
mcp-proxy convert https://api.example.com/mcp/sse \
  --auth "Bearer your-api-token"

# Add custom headers
mcp-proxy convert https://api.example.com/mcp/sse \
  -H "Authorization=Bearer token" \
  -H "X-Custom-Header=value"
```

### Use with MCP Clients

```bash
# Pipe mcp-proxy output to your MCP client
mcp-proxy convert https://remote-server.com/mcp \
  --auth "Bearer token" | \
  your-mcp-client

# Or use in MCP client configuration
# Example (Claude Desktop configuration):
{
  "mcpServers": {
    "remote-service": {
      "command": "mcp-proxy",
      "args": [
        "convert",
        "https://remote-server.com/mcp/sse",
        "--auth",
        "Bearer your-token"
      ]
    }
  }
}
```

## Command Details

### 1. `convert` - Protocol Conversion (Core Command)

Convert remote MCP service to local stdio interface.

```bash
mcp-proxy convert <URL> [options]
```

**Options:**

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--auth <TOKEN>` | `-a` | Authentication header (e.g., "Bearer token") | - |
| `--header <KEY=VALUE>` | `-H` | Custom HTTP headers (can be used multiple times) | - |
| `--timeout <SECONDS>` | - | Connection timeout in seconds | 30 |
| `--retries <NUM>` | - | Number of retries | 3 |
| `--verbose` | `-v` | Verbose output (show debug info) | false |
| `--quiet` | `-q` | Quiet mode (errors only) | false |

**Examples:**

```bash
# Basic conversion
mcp-proxy convert https://api.example.com/mcp/sse

# With authentication and timeout
mcp-proxy convert https://api.example.com/mcp/sse \
  --auth "Bearer sk-1234567890" \
  --timeout 60 \
  --retries 5

# Add multiple custom headers
mcp-proxy convert https://api.example.com/mcp \
  -H "Authorization=Bearer token" \
  -H "X-API-Key=your-key" \
  -H "X-Request-ID=abc123"

# Verbose mode (view connection process)
mcp-proxy convert https://api.example.com/mcp/sse --verbose
```

### 2. `check` - Service Status Check

Check if remote MCP service is available, verify connectivity and protocol support.

```bash
mcp-proxy check <URL> [options]
```

**Options:**

| Option | Short | Description | Default |
|--------|-------|-------------|---------|
| `--auth <TOKEN>` | `-a` | Authentication header | - |
| `--timeout <SECONDS>` | - | Timeout in seconds | 10 |

**Examples:**

```bash
# Check service status
mcp-proxy check https://api.example.com/mcp/sse

# With authentication
mcp-proxy check https://api.example.com/mcp/sse \
  --auth "Bearer token" \
  --timeout 5
```

**Exit Codes:**
- `0`: Service is healthy
- `Non-zero`: Service unavailable or check failed

### 3. `detect` - Protocol Detection

Automatically detect the protocol type used by remote MCP service.

```bash
mcp-proxy detect <URL> [options]
```

**Options:**

| Option | Short | Description |
|--------|-------|-------------|
| `--auth <TOKEN>` | `-a` | Authentication header |
| `--quiet` | `-q` | Quiet mode (output protocol type only) |

**Output:**
- `SSE` - Server-Sent Events protocol
- `Streamable HTTP` - Streamable HTTP protocol
- `Stdio` - Standard input/output protocol (not applicable for remote services)

**Examples:**

```bash
# Detect protocol type
mcp-proxy detect https://api.example.com/mcp/sse

# Use in scripts
PROTOCOL=$(mcp-proxy detect https://api.example.com/mcp --quiet)
if [ "$PROTOCOL" = "SSE" ]; then
  echo "Detected SSE protocol"
fi
```

## Use Cases

### Case 1: Claude Desktop with Remote MCP Service

Claude Desktop only supports stdio protocol MCP services. Use `mcp-proxy` to access remote services.

**Configuration Example** (`~/Library/Application Support/Claude/config.json`):

```json
{
  "mcpServers": {
    "remote-database": {
      "command": "mcp-proxy",
      "args": [
        "convert",
        "https://your-server.com/mcp/database",
        "--auth",
        "Bearer your-token-here"
      ]
    },
    "remote-search": {
      "command": "mcp-proxy",
      "args": ["https://search-api.com/mcp/sse"]
    }
  }
}
```

### Case 2: Health Check in CI/CD Pipeline

```bash
#!/bin/bash
# Check MCP service status before deployment

echo "Checking MCP service..."
if mcp-proxy check https://api.example.com/mcp --timeout 5; then
  echo "✅ MCP service is healthy, continuing deployment"
  # Execute deployment script
  ./deploy.sh
else
  echo "❌ MCP service unavailable, aborting deployment"
  exit 1
fi
```

### Case 3: Cross-Network Enterprise Internal MCP Service

```bash
# Access internal MCP service via VPN or jump host
mcp-proxy convert https://internal-mcp.company.com/api/sse \
  --auth "Bearer ${MCP_TOKEN}" \
  --timeout 120 | \
  local-mcp-client
```

### Case 4: Development and Testing

```bash
# Quick test remote MCP service
mcp-proxy convert https://test-api.com/mcp/sse --verbose

# View detailed connection and communication logs
RUST_LOG=debug mcp-proxy convert https://api.com/mcp/sse -v
```

## Supported Protocols

`mcp-proxy` can connect to remote MCP services using the following protocols:

| Protocol | Description | Status |
|----------|-------------|--------|
| **SSE** | Server-Sent Events, unidirectional real-time push | ✅ Fully Supported |
| **Streamable HTTP** | Bidirectional streaming HTTP communication | ✅ Fully Supported |

**Output Protocol**: Always **stdio** (standard input/output)

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `RUST_LOG` | Log level | `RUST_LOG=debug mcp-proxy convert ...` |
| `HTTP_PROXY` | HTTP proxy | `HTTP_PROXY=http://proxy:8080` |
| `HTTPS_PROXY` | HTTPS proxy | `HTTPS_PROXY=http://proxy:8080` |

## FAQ

### Q: Why do I need mcp-proxy?

**A:** Many MCP clients (like Claude Desktop) only support local stdio protocol services. If your MCP service is deployed on a remote server using SSE or HTTP protocols, you need `mcp-proxy` as a protocol conversion bridge.

### Q: What's the difference between mcp-proxy and MCP server?

**A:**
- **MCP Server**: Backend service that provides specific functionality (database access, file operations, etc.)
- **mcp-proxy**: Pure client proxy tool that only does protocol conversion, provides no business functionality

### Q: Does it support bidirectional communication?

**A:** Yes! Whether using SSE or Streamable HTTP protocol, `mcp-proxy` supports full bidirectional communication (request/response).

### Q: How to debug connection issues?

**A:** Use `--verbose` option and `RUST_LOG` environment variable:

```bash
RUST_LOG=debug mcp-proxy convert https://api.com/mcp --verbose
```

### Q: Does it support self-signed SSL certificates?

**A:** Current version uses system default certificate verification. For self-signed certificate support, please submit an Issue.

## Troubleshooting

### Connection Timeout

```bash
# Increase timeout
mcp-proxy convert https://slow-api.com/mcp --timeout 120
```

### Authentication Failed

```bash
# Check token format, ensure "Bearer " prefix
mcp-proxy convert https://api.com/mcp --auth "Bearer your-token-here"

# Or use custom header
mcp-proxy convert https://api.com/mcp -H "Authorization=Bearer your-token"
```

### Protocol Detection Failed

```bash
# View detailed error message
mcp-proxy detect https://api.com/mcp --verbose

# Check service status
mcp-proxy check https://api.com/mcp
```

## System Requirements

- **Operating System**: Linux, macOS, Windows
- **Rust Version**: 1.70+ (only required for building from source)
- **Network**: Ability to access target MCP service

## License

This project is dual-licensed under MIT or Apache-2.0.

## Contributing

Issues and Pull Requests are welcome!

- **GitHub Repository**: https://github.com/nuwax-ai/mcp-proxy
- **Issue Tracker**: https://github.com/nuwax-ai/mcp-proxy/issues
- **Feature Discussions**: https://github.com/nuwax-ai/mcp-proxy/discussions

## Related Resources

- [MCP Official Documentation](https://modelcontextprotocol.io/)
- [rmcp - Rust MCP Implementation](https://crates.io/crates/rmcp)
- [MCP Servers List](https://github.com/modelcontextprotocol/servers)

## Changelog

### v0.1.18

- ✅ SSE and Streamable HTTP protocol conversion support
- ✅ Auto protocol detection
- ✅ Authentication and custom headers support
- ✅ Service status check command
- ✅ Protocol detection command
- ✅ OpenTelemetry integration with OTLP
- ✅ Background health checks
- ✅ Run code execution via external processes
