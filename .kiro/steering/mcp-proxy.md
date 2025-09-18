---
inclusion: fileMatch
fileMatchPattern: ['mcp-proxy/**/*.rs']
---

# MCP Proxy Guidelines

## MCP Protocol Implementation
- Use `rmcp` crate for MCP protocol handling
- Support multiple transport layers: SSE, HTTP, child process, IO
- Implement proper protocol versioning and capability negotiation
- Handle connection lifecycle and reconnection logic

## Dynamic Routing
- Implement dynamic route registration and removal
- Use `DashMap` for thread-safe route storage
- Support route pattern matching and parameter extraction
- Provide route health checking and automatic cleanup

## Code Execution
- Use `run_code_rmcp` for sandboxed code execution
- Support JavaScript, TypeScript, and Python execution
- Implement proper resource limits (memory, CPU, time)
- Handle execution environment isolation and cleanup

## SSE (Server-Sent Events)
- Implement SSE server for real-time communication
- Handle client connection management and cleanup
- Support event filtering and subscription management
- Provide proper error handling and reconnection logic

## Service Health Monitoring
- Implement automatic health checks for registered services
- Handle service failure detection and recovery
- Provide service status reporting via API
- Support service dependency tracking

## Middleware Architecture
- Use `tower` middleware for cross-cutting concerns
- Implement request/response logging with structured data
- Add request ID tracking for distributed tracing
- Support authentication and authorization middleware

## Configuration Management
- Support dynamic MCP service configuration
- Implement configuration validation and hot-reloading
- Handle service registration and discovery
- Provide configuration templates and examples

## Background Tasks
- Use `tokio::spawn` for background service management
- Implement scheduled health checks and cleanup tasks
- Handle task cancellation and graceful shutdown
- Provide task monitoring and status reporting