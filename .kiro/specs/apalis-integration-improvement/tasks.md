# Implementation Plan

**CRITICAL STATUS**: The current apalis integration implementation has multiple compilation errors and does not work. The existing code in `apalis_sqlite.rs`, `apalis_transcription.rs`, and related modules needs significant fixes before any new features can be added. Priority should be on fixing compilation errors and getting a basic working apalis integration before adding advanced features.

- [x] 1. Set up apalis dependencies and basic configuration

  - Add apalis-sql, sqlx, and related dependencies to voice-cli Cargo.toml
  - Create apalis configuration structure in existing config system
  - Add feature flag for apalis vs custom task processing
  - _Requirements: 7.1, 7.4, 7.5_

- [x] 1.1 Research and understand current apalis API patterns

  - Study apalis documentation for correct stepped workflow implementation
  - Understand proper Job trait implementation for custom types
  - Research correct SqliteStorage setup and usage patterns
  - Identify proper WorkerBuilder API for stepped tasks
  - Document correct patterns for Data context access and error handling
  - _Requirements: All subsequent tasks depend on this research_

- [x] 2. Fix critical apalis SQLite storage manager implementation

  - Fix ApalisSqliteManager::setup() method to use correct apalis-sql API
  - Replace broken start_stepped() calls with proper apalis job submission
  - Fix SqliteStorage generic type parameters to work with stepped jobs
  - Correct database connection setup and table creation for apalis schema
  - Remove compilation errors in apalis_sqlite.rs module
  - _Requirements: 1.1, 1.2, 1.3, 1.5_

- [ ] 3. Fix critical apalis job type compatibility issues

  - Research and implement correct apalis Job trait for AsyncTranscriptionTask
  - Fix step job types to work with apalis stepped workflow requirements
  - Resolve serialization and type compatibility compilation errors
  - Correct step function signatures to match current apalis API
  - Fix all compilation errors in stepped_task.rs and related modules
  - _Requirements: 3.1, 3.2, 3.3, 3.5_

- [x] 4. Fix critical stepped task function compilation errors
- [x] 4.1 Fix audio format processing step function compilation

  - Fix Data<Arc<TranscriptionContext>> access pattern (field `0` is private error)
  - Correct apalis Error type conversion from anyhow::Error
  - Fix GoTo return types to match apalis step requirements
  - Resolve all compilation errors in audio_format_step_apalis function
  - _Requirements: 2.2, 6.1, 6.5_

- [x] 4.2 Fix transcription processing step function compilation

  - Fix Data context access to resolve private field errors
  - Correct apalis error handling and type conversion issues
  - Fix step transitions and GoTo return type mismatches
  - Resolve all compilation errors in whisper_transcription_step_apalis
  - _Requirements: 2.2, 6.2, 6.5_

- [x] 4.3 Fix result formatting step function compilation

  - Fix final step return type to match apalis requirements
  - Correct error handling and type conversion issues
  - Fix GoTo::Done usage with proper result types
  - Resolve all compilation errors in result_formatting_step_apalis
  - _Requirements: 2.2, 6.3, 6.5_

- [ ] 5. Fix critical apalis worker management compilation errors
- [ ] 5.1 Fix WorkerBuilder build_stepped() compilation errors

  - Research correct apalis WorkerBuilder API for stepped workflows
  - Fix storage backend setup to work with SqliteStorage<AsyncTranscriptionTask>
  - Resolve trait bound errors for SteppableStorage and Backend traits
  - Fix concurrency configuration field name (max_concurrent_tasks not found)
  - Resolve all compilation errors in setup_apalis_transcription_worker
  - _Requirements: 4.2, 4.6_

- [ ] 5.2 Fix worker lifecycle management

  - Fix worker startup and shutdown procedures
  - Correct error handling in worker management
  - Fix worker statistics and monitoring integration
  - Add proper graceful shutdown handling
  - _Requirements: 4.1, 4.3, 4.4, 4.5_

- [ ] 6. Create HTTP API endpoints for task management
- [ ] 6.1 Implement POST /tasks/transcribe endpoint

  - Create task submission handler that uses start_stepped() method
  - Return job ID for tracking using apalis job identification
  - Validate input parameters and create AsyncTranscriptionTask
  - Add proper error responses for invalid requests
  - _Requirements: 5.1, 5.6_

- [ ] 6.2 Implement GET /tasks/{job_id} status endpoint

  - Create job status handler that queries apalis SQLite storage
  - Return job state (Pending, Running, Done, Failed, Killed) from apalis
  - Include current step name and progress information
  - Add completion time and error details when available
  - _Requirements: 5.2, 5.3, 5.4, 5.5, 8.1, 8.2, 8.6_

- [ ] 6.3 Implement DELETE /tasks/{job_id} cancellation endpoint

  - Create job cancellation handler using apalis job management
  - Update job status to "Killed" and stop processing gracefully
  - Return appropriate response confirming cancellation
  - Handle cases where job is already completed or failed
  - _Requirements: 5.3, 8.5_

- [ ] 6.4 Implement GET /tasks listing endpoint

  - Create job listing handler with filtering by status and priority
  - Add pagination support for large job lists
  - Query apalis storage with appropriate filters and limits
  - Return job metadata and current status for each job
  - _Requirements: 5.4, 8.6, 8.7_

- [ ] 7. Add job state tracking and monitoring

  - Implement job metadata storage for extended tracking information
  - Add step progress tracking within apalis job context
  - Create job history and retry count tracking
  - Implement job cleanup for completed and failed jobs based on configuration
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.7, 8.8_

- [ ] 8. Integrate with existing transcription services

  - Update TranscriptionContext to include all existing services
  - Ensure AudioFileManager integration works with stepped workflow
  - Maintain TaskStore compatibility for job metadata
  - Preserve existing error handling patterns and types
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5_

- [ ] 9. Add configuration management and feature flags

  - Extend existing config system with apalis-specific settings
  - Add feature flag to switch between apalis and custom task processing
  - Implement configuration validation for apalis settings
  - Add environment variable overrides for apalis configuration
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 9.3, 9.4_

- [ ] 10. Implement backward compatibility layer

  - Ensure existing /transcribe endpoint continues to work unchanged
  - Add fallback mechanism when apalis is disabled via configuration
  - Implement concurrent operation of both systems without interference
  - Maintain existing synchronous API response formats
  - _Requirements: 9.1, 9.2, 9.5, 9.6_

- [ ] 11. Add comprehensive error handling and logging

  - Implement ApalisIntegrationError types with proper error conversion
  - Add structured logging for apalis job lifecycle events
  - Implement error recovery strategies for different failure types
  - Add monitoring and alerting for job processing issues
  - _Requirements: 2.3, 4.3, 8.3, 8.4, 8.8_

- [ ] 12. Create tests for apalis integration
- [ ] 12.1 Write unit tests for step functions

  - Test each step function in isolation with mock dependencies
  - Verify proper error handling and edge cases for each step
  - Test job data transformation between steps
  - Validate apalis GoTo return types and step transitions
  - _Requirements: 2.1, 2.2, 2.3, 3.4_

- [ ] 12.2 Write integration tests for worker and storage

  - Test complete stepped workflow from submission to completion
  - Verify job state transitions and apalis storage operations
  - Test worker concurrency and job coordination
  - Test job cancellation and cleanup operations
  - _Requirements: 4.1, 4.2, 4.4, 5.3, 8.5_

- [ ] 12.3 Write API integration tests
  - Test all /tasks endpoints with various scenarios
  - Verify job submission, status tracking, and cancellation
  - Test error responses and edge cases
  - Validate backward compatibility with existing endpoints
  - _Requirements: 5.1, 5.2, 5.3, 5.4, 9.1, 9.2_
