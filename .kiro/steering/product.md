---
inclusion: always
---

# Product Overview

## RMCP-PROXY

A comprehensive document processing and MCP (Model Context Protocol) proxy service built in Rust. The project consists of two main components:

### Document Parser Service
- **Purpose**: Multi-format document parsing service that converts PDF, Word, Excel, PowerPoint, images, and audio files into structured Markdown
- **Key Features**: 
  - Dual-engine parsing (MinerU for PDFs, MarkItDown for other formats)
  - Automatic format detection and engine selection
  - Real-time Markdown structure processing with table of contents generation
  - OSS storage integration for processed documents and images
  - Asynchronous task processing with status tracking

### MCP Proxy Service  
- **Purpose**: MCP proxy service enabling remote access to MCP functionality via SSE (Server-Sent Events)
- **Key Features**:
  - Dynamic MCP plugin configuration and loading
  - SSE protocol support for real-time communication
  - Code execution capabilities (JavaScript, TypeScript, Python)
  - Automatic service health monitoring and management
  - RESTful API for MCP service management

## Target Users
- Developers needing document processing capabilities
- Systems requiring MCP protocol integration
- Applications needing real-time document structure analysis
- Services requiring multi-format document conversion to Markdown

## Core Value Proposition
- **Zero-dependency deployment**: Single binary deployment with automatic environment management
- **Multi-format support**: Comprehensive document format coverage
- **Real-time processing**: Synchronous Markdown processing with instant structured output
- **Production-ready**: Built with Rust for performance, safety, and reliability