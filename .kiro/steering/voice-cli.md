---
inclusion: fileMatch
fileMatchPattern: ["voice-cli/**/*.rs"]
---

# Voice CLI Development Guidelines

## Architecture Overview

Voice CLI is an async transcription service built with Rust, featuring stepped task processing, background services, and RESTful APIs. The service processes audio files through a multi-stage pipeline with recovery capabilities.

## Task Processing Pipeline

### Stepped Task Architecture
- **Three-step pipeline**: Audio format detection/conversion → Whisper transcription → Result formatting
- **Task types**: `AsyncTranscriptionTask` → `AudioProcessedTask` → `TranscriptionCompletedTask`
- **Step functions**: `audio_format_step()`, `whisper_transcription_step()`, `result_formatting_step()`
- Each step is idempotent and recoverable with proper error handling
- Use `ProcessingStage` enum to track current step and progress

### Task Queue Management
- Use `apalis` with SQLite backend for persistent task queues
- Implement `SteppedTranscriptionWorker` for concurrent task processing
- Use `mpsc::UnboundedSender/Receiver` for internal task distribution
- Limit concurrency with `tokio::sync::Semaphore`
- Store task state in `TaskStore` with status tracking

### Error Handling & Recovery
- Use `TaskError` enum with `is_recoverable()` method
- Implement retry logic for recoverable errors
- Store intermediate results for step recovery
- Clean up temporary files on both success and failure paths

## Audio Processing

### Format Detection & Conversion
- Use `symphonia` for audio format detection and metadata extraction
- Support formats: WAV, MP3, FLAC, OGG, M4A, AAC
- Implement `AudioFormatDetector` service for format validation
- Convert to WAV format for Whisper processing when needed

### Speech-to-Text Integration
- Use `voice-toolkit` for Whisper model integration
- Support multiple Whisper models (base, small, medium, large)
- Implement `TranscriptionEngine` service for model management
- Convert between internal types and `voice-toolkit` types

## Service Architecture

### Background Services
- Implement `BackgroundService` trait for service lifecycle management
- Use `tokio::spawn` for concurrent service execution
- Handle graceful shutdown with `tokio::signal::ctrl_c()`
- Implement health monitoring and automatic restart capabilities

### Core Services
- **ModelService**: Whisper model management and loading
- **TranscriptionEngine**: Speech-to-text processing coordination
- **TaskStore**: Task persistence and status tracking
- **AudioFileManager**: File handling and cleanup
- **SteppedTranscriptionWorker**: Main task processing orchestrator

### Service Context
- Use `TranscriptionContext` to share services between steps
- Wrap shared state in `Arc<T>` for thread safety
- Pass context through all processing steps

## API Design

### HTTP Endpoints
- **POST /transcribe**: Submit transcription tasks (multipart/form-data)
- **GET /tasks/{id}/status**: Get task status and progress
- **DELETE /tasks/{id}**: Cancel running tasks
- **GET /health**: Service health check
- **GET /models**: List available Whisper models

### Response Patterns
- Use `HttpResult<T>` wrapper for consistent API responses
- Include `request_id` for request tracing
- Implement proper HTTP status codes (200, 202, 404, 500)
- Use `utoipa` for OpenAPI documentation generation

### Real-time Updates
- Implement Server-Sent Events (SSE) for task progress updates
- Use `TaskStatus` enum with detailed progress information
- Include `ProgressDetails` with stage progress and time estimates

## Configuration Management

### Config Structure
- Use `Config` struct with nested configuration sections
- Support YAML files with environment variable overrides
- Implement `TaskManagementConfig`, `ServerConfig`, `TranscriptionConfig`
- Validate configuration at startup with descriptive error messages

### Environment Variables
- Prefix all env vars with `VOICE_CLI_`
- Support database URL, server port, model paths, log levels
- Provide sensible defaults for development

## Storage & Persistence

### Task Storage
- Use `sled` embedded database for task metadata and status
- Implement `TaskStore` with async methods for CRUD operations
- Store serialized task data with JSON encoding
- Handle concurrent access with proper locking

### File Management
- Store uploaded audio files in configurable directory
- Use UUID-based filenames to avoid conflicts
- Implement automatic cleanup of processed files
- Track cleanup files in task metadata

## Testing Patterns

### Unit Tests
- Test each service in isolation with mocked dependencies
- Use `tempfile::TempDir` for temporary test databases
- Create test fixtures for audio files and configurations
- Test error conditions and recovery scenarios

### Integration Tests
- Test complete task processing pipeline
- Verify API endpoints with real HTTP requests
- Test service startup and shutdown sequences
- Validate configuration loading and validation

## Error Handling

### Error Types
- Use `VoiceCliError` enum for application-specific errors
- Implement `From` traits for error conversion
- Include context information in error messages
- Distinguish between recoverable and non-recoverable errors

### Logging & Monitoring
- Use `tracing` for structured logging with spans
- Log task lifecycle events with appropriate levels
- Include task IDs in all related log messages
- Implement request tracing with correlation IDs

## Performance Considerations

### Concurrency
- Limit concurrent transcription tasks based on system resources
- Use async/await throughout for non-blocking I/O
- Implement backpressure in task queue when overloaded
- Monitor memory usage during large file processing

### Resource Management
- Clean up temporary files promptly after processing
- Implement connection pooling for database access
- Cache loaded Whisper models to avoid repeated loading
- Monitor disk space for audio file storage
