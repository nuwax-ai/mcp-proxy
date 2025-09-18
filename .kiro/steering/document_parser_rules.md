---
inclusion: fileMatch
fileMatchPattern: ['document-parser/**/*.rs', 'oss-client/**/*.rs']
---

# Document Processing Guidelines

## Concurrent Data Structures
- Use `DashMap` instead of `RwLock<HashMap>` for thread-safe maps
- Prefer lock-free data structures for high-concurrency scenarios
- Use `Arc<DashMap<K, V>>` for shared concurrent access

## Document Parsing Engines
- **MinerU**: Primary engine for PDF processing (Python integration)
- **MarkItDown**: Multi-format parsing for Word, Excel, PowerPoint, images
- **Dual-engine coordination**: Automatic format detection and engine selection
- Handle Python environment management gracefully

## File Processing Patterns
- Stream large files to avoid memory exhaustion
- Use temporary files for intermediate processing steps
- Implement proper cleanup with `scopeguard` or RAII patterns
- Validate file formats before processing

## OSS Integration
- Use `aliyun-oss-rust-sdk` for cloud storage operations
- Implement signed URL generation for secure file access
- Handle upload/download with proper error recovery
- Cache frequently accessed files locally

## Markdown Processing
- Use `pulldown-cmark` for parsing and manipulation
- Generate table of contents with `pulldown-cmark-toc`
- Preserve document structure and metadata
- Handle image references and embedded content

## Task Management
- Implement asynchronous processing with status tracking
- Use `uuid::v7` for time-ordered task identifiers
- Provide real-time progress updates via SSE
- Handle task cancellation and cleanup gracefully