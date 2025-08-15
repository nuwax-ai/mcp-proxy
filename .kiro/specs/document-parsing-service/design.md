# Design Document

## Overview

The Document Parsing Service is a high-performance, multi-format document processing system built with Rust and Axum. It implements a dual-engine architecture combining MinerU (for PDF processing) and MarkItDown (for other formats) to convert various document types into structured Markdown content. The system provides both asynchronous document processing with comprehensive task tracking and synchronous Markdown structuring capabilities.

The service is designed as a subproject within a Rust workspace, following modern Rust best practices with comprehensive error handling, automatic environment management, and cloud storage integration.

## Architecture

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Document Parsing Service                     │
├─────────────────────────────────────────────────────────────────┤
│  HTTP API Layer (Axum)                                         │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Document Handler│ │ Task Handler    │ │ Markdown Handler│   │
│  │ - Upload        │ │ - Status Query  │ │ - Download      │   │
│  │ - URL Submit    │ │ - Task List     │ │ - Sync Process  │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Service Layer                                                  │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Document Service│ │ Task Queue      │ │ Storage Service │   │
│  │ - Orchestration │ │ - Concurrency   │ │ - Sled DB       │   │
│  │ - Validation    │ │ - Job Dispatch  │ │ - Caching       │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Processing Layer                                               │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Format Detector │ │ Dual Engine     │ │ Markdown Proc.  │   │
│  │ - MIME Detection│ │ - MinerU (PDF)  │ │ - TOC Generation│   │
│  │ - Extension Map │ │ - MarkItDown    │ │ - Section Split │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│  Infrastructure Layer                                           │
│  ┌─────────────────┐ ┌─────────────────┐ ┌─────────────────┐   │
│  │ Environment Mgr │ │ OSS Service     │ │ Utils & Logging │   │
│  │ - Python Setup  │ │ - S3 Compatible │ │ - Health Check  │   │
│  │ - Dependency    │ │ - File Upload   │ │ - Metrics       │   │
│  └─────────────────┘ └─────────────────┘ └─────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

### Component Interaction Flow

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│   Client    │───▶│  HTTP API   │───▶│   Service   │───▶│  Processing │
│             │    │   Layer     │    │    Layer    │    │    Layer    │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
       ▲                   │                   │                   │
       │                   ▼                   ▼                   ▼
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│   Response  │◄───│  Task Queue │◄───│  Storage    │◄───│ Environment │
│   Handler   │    │  Management │    │  Services   │    │  Manager    │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
```

## Components and Interfaces

### 1. HTTP API Layer

#### Document Handler (`src/handlers/document_handler.rs`)
- **Purpose**: Handle document upload and URL submission requests
- **Key Methods**:
  - `upload_document()`: Process multipart file uploads
  - `submit_url()`: Handle URL-based document submission
  - `validate_request()`: Input validation and sanitization

#### Task Handler (`src/handlers/task_handler.rs`)
- **Purpose**: Manage task lifecycle and status queries
- **Key Methods**:
  - `get_task_status()`: Return detailed task status
  - `list_tasks()`: Paginated task listing with filters
  - `cancel_task()`: Cancel running tasks

#### Markdown Handler (`src/handlers/markdown_handler.rs`)
- **Purpose**: Handle Markdown processing and download
- **Key Methods**:
  - `download_markdown()`: Stream processed Markdown files
  - `get_oss_url()`: Generate temporary download URLs
  - `process_markdown_sync()`: Synchronous Markdown structuring

### 2. Service Layer

#### Document Service (`src/services/document_service.rs`)
- **Purpose**: Orchestrate document processing workflow
- **Key Responsibilities**:
  - Format detection and engine selection
  - Processing pipeline coordination
  - Result aggregation and storage
- **Interface**:
```rust
pub trait DocumentService {
    async fn process_document(&self, task: DocumentTask) -> Result<ProcessResult>;
    async fn get_processing_status(&self, task_id: &str) -> Result<TaskStatus>;
    async fn cancel_processing(&self, task_id: &str) -> Result<()>;
}
```

#### Task Queue Service (`src/services/task_queue_service.rs`)
- **Purpose**: Manage concurrent task processing
- **Key Features**:
  - Channel-based task distribution
  - Configurable concurrency limits
  - Automatic retry with exponential backoff
- **Interface**:
```rust
pub trait TaskQueueService {
    async fn enqueue_task(&self, task: DocumentTask) -> Result<()>;
    async fn get_queue_stats(&self) -> QueueStatistics;
    async fn shutdown_gracefully(&self) -> Result<()>;
}
```

#### Storage Service (`src/services/storage_service.rs`)
- **Purpose**: Manage persistent data storage
- **Key Features**:
  - Sled embedded database integration
  - Task metadata persistence
  - Automatic data expiration
- **Interface**:
```rust
pub trait StorageService {
    async fn save_task(&self, task: &DocumentTask) -> Result<()>;
    async fn get_task(&self, task_id: &str) -> Result<Option<DocumentTask>>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<()>;
    async fn cleanup_expired_tasks(&self) -> Result<usize>;
}
```

### 3. Processing Layer

#### Format Detector (`src/parsers/format_detector.rs`)
- **Purpose**: Intelligent document format detection
- **Detection Methods**:
  - File extension analysis
  - MIME type detection
  - File header magic number analysis
- **Interface**:
```rust
pub trait FormatDetector {
    fn detect_format(&self, file_path: &Path, mime_type: Option<&str>) -> DocumentFormat;
    fn select_parser_engine(&self, format: &DocumentFormat) -> ParserEngine;
}
```

#### Dual Engine Parser (`src/parsers/dual_engine_parser.rs`)
- **Purpose**: Unified interface for both parsing engines
- **Engine Selection**:
  - PDF files → MinerU engine
  - Other formats → MarkItDown engine
- **Interface**:
```rust
#[async_trait]
pub trait DocumentParser {
    async fn parse_document(&self, file_path: &Path) -> Result<ParseResult>;
    async fn get_supported_formats(&self) -> Vec<DocumentFormat>;
    async fn validate_environment(&self) -> Result<()>;
}
```

#### Markdown Processor (`src/processors/markdown_processor.rs`)
- **Purpose**: Process and structure Markdown content
- **Key Features**:
  - TOC generation using pulldown-cmark-toc
  - Content sectioning by headings
  - Image path resolution and replacement
- **Interface**:
```rust
pub trait MarkdownProcessor {
    fn process_markdown(&self, content: &str) -> Result<StructuredDocument>;
    fn generate_toc(&self, content: &str) -> Result<Vec<TocItem>>;
    fn split_by_sections(&self, content: &str, toc: &[TocItem]) -> Result<HashMap<String, String>>;
}
```

### 4. Infrastructure Layer

#### Environment Manager (`src/utils/environment_manager.rs`)
- **Purpose**: Automatic Python environment setup and validation
- **Key Features**:
  - UV package manager integration
  - MinerU and MarkItDown installation
  - CUDA environment detection
- **Interface**:
```rust
pub trait EnvironmentManager {
    async fn check_environment(&self) -> Result<EnvironmentStatus>;
    async fn setup_python_environment(&self) -> Result<()>;
    async fn install_dependencies(&self) -> Result<()>;
    async fn validate_engines(&self) -> Result<()>;
}
```

#### OSS Service (`src/services/oss_service.rs`)
- **Purpose**: Cloud storage integration
- **Key Features**:
  - S3-compatible API support
  - Batch file uploads
  - Temporary URL generation
- **Interface**:
```rust
pub trait OssService {
    async fn upload_file(&self, file_path: &Path, key: &str) -> Result<String>;
    async fn upload_batch(&self, files: Vec<(PathBuf, String)>) -> Result<Vec<String>>;
    async fn generate_download_url(&self, key: &str, expires_in: Duration) -> Result<String>;
    async fn file_exists(&self, key: &str) -> Result<bool>;
}
```

## Data Models

### Core Data Structures

#### DocumentTask (`src/models/document_task.rs`)
```rust
pub struct DocumentTask {
    pub id: String,
    pub status: TaskStatus,
    pub source_type: SourceType,
    pub source_path: Option<String>,
    pub document_format: DocumentFormat,
    pub parser_engine: ParserEngine,
    pub progress: u32,
    pub error_message: Option<String>,
    pub oss_data: Option<OssData>,
    pub structured_document: Option<StructuredDocument>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
}
```

#### StructuredDocument (`src/models/structured_document.rs`)
```rust
pub struct StructuredDocument {
    pub task_id: String,
    pub document_title: String,
    pub toc: Vec<StructuredSection>,
    pub total_sections: usize,
    pub last_updated: DateTime<Utc>,
    pub word_count: Option<usize>,
    pub processing_time: Option<String>,
}

pub struct StructuredSection {
    pub id: String,
    pub title: String,
    pub level: u8,
    pub content: String,
    pub children: Vec<StructuredSection>,
    pub is_edited: Option<bool>,
    pub word_count: Option<usize>,
    pub start_pos: Option<usize>,
    pub end_pos: Option<usize>,
}
```

#### TaskStatus (`src/models/task_status.rs`)
```rust
pub enum TaskStatus {
    Pending,
    Processing { stage: ProcessingStage },
    Completed,
    Failed { error: String },
    Cancelled,
}

pub enum ProcessingStage {
    DownloadingDocument,
    FormatDetection,
    MinerUExecuting,
    MarkItDownExecuting,
    UploadingImages,
    ProcessingMarkdown,
    GeneratingToc,
    SplittingContent,
    UploadingMarkdown,
}
```

### Configuration Models

#### AppConfig (`src/config.rs`)
```rust
pub struct AppConfig {
    pub server: ServerConfig,
    pub log: LogConfig,
    pub document_parser: DocumentParserConfig,
    pub mineru: MinerUConfig,
    pub markitdown: MarkItDownConfig,
    pub storage: StorageConfig,
    pub external_integration: ExternalIntegrationConfig,
}
```

## Error Handling

### Error Strategy
The system uses a layered error handling approach following Rust best practices:

1. **Application Layer**: Uses `anyhow` for flexible error handling and context
2. **Library Layer**: Uses `thiserror` for structured, typed errors
3. **HTTP Layer**: Converts all errors to standardized HTTP responses

### Error Types (`src/error.rs`)
```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),
    #[error("文件操作错误: {0}")]
    File(String),
    #[error("不支持的文件格式: {0}")]
    UnsupportedFormat(String),
    #[error("解析错误: {0}")]
    Parse(String),
    #[error("MinerU错误: {0}")]
    MinerU(String),
    #[error("MarkItDown错误: {0}")]
    MarkItDown(String),
    // ... additional error variants
}
```

### Error Response Format
```rust
pub struct HttpResult<T> {
    pub code: String,
    pub message: String,
    pub data: Option<T>,
}
```

## Testing Strategy

### Unit Testing
- **Coverage Target**: >80% code coverage
- **Test Organization**: Tests co-located with source code using `#[cfg(test)]`
- **Mock Strategy**: Use trait objects and dependency injection for testability

### Integration Testing
- **API Testing**: Full HTTP request/response cycle testing
- **Database Testing**: Sled database operations with temporary databases
- **Engine Testing**: MinerU and MarkItDown integration with sample documents

### Performance Testing
- **Benchmarks**: Criterion-based benchmarks for critical paths
- **Load Testing**: Concurrent request handling validation
- **Memory Testing**: Memory usage profiling and leak detection

### Test Structure
```
tests/
├── integration/
│   ├── api_tests.rs
│   ├── engine_tests.rs
│   └── storage_tests.rs
├── fixtures/
│   ├── sample.pdf
│   ├── sample.docx
│   └── sample.md
└── common/
    └── test_utils.rs
```

## Deployment Architecture

### Single Binary Deployment
- **Compilation**: Optimized release build with native CPU features
- **Dependencies**: All Rust dependencies statically linked
- **Python Environment**: Automatically managed via UV

### Environment Management
- **Automatic Setup**: Python environment creation and dependency installation
- **Health Checks**: Comprehensive environment validation
- **Graceful Degradation**: Service continues with available engines

### Configuration Management
- **Hierarchy**: YAML file → Environment variables → Defaults
- **Validation**: Comprehensive configuration validation at startup
- **Hot Reload**: Configuration changes detected and applied

### Monitoring and Observability
- **Structured Logging**: JSON-formatted logs with correlation IDs
- **Metrics**: Prometheus-compatible metrics export
- **Health Endpoints**: Detailed health check information
- **Alerting**: Configurable alert rules with multiple notification channels

## Security Considerations

### Input Validation
- **File Type Validation**: MIME type and extension verification
- **Size Limits**: Configurable file size restrictions
- **Content Scanning**: Basic malware detection capabilities

### Data Protection
- **Sensitive Data**: Automatic sanitization in logs
- **Temporary Files**: Secure cleanup of processing artifacts
- **Access Control**: API key-based authentication

### Network Security
- **TLS Support**: HTTPS endpoint configuration
- **Rate Limiting**: Request throttling and abuse prevention
- **CORS**: Configurable cross-origin resource sharing

## Performance Optimization

### Concurrency Model
- **Async Runtime**: Tokio-based async/await throughout
- **Task Queue**: Channel-based work distribution
- **Resource Limits**: Configurable concurrency bounds

### Memory Management
- **Streaming**: Large file processing with streaming I/O
- **Caching**: Intelligent caching of frequently accessed data
- **Cleanup**: Automatic cleanup of expired data and temporary files

### Database Optimization
- **Embedded Storage**: Sled for high-performance local storage
- **Indexing**: Efficient task lookup and filtering
- **Compaction**: Automatic database maintenance

This design provides a robust, scalable foundation for the document parsing service while maintaining the flexibility to extend functionality and adapt to changing requirements.