# Requirements Document

## Introduction

This feature improves the current apalis integration in the voice-cli service by fixing existing implementation issues and properly implementing the apalis stepped workflow pattern. The current implementation has several problems with the apalis API usage, including incorrect storage setup methods, improper job type definitions, and missing worker management functionality. This enhancement will correct these issues to work with the latest apalis version while maintaining backward compatibility with existing synchronous endpoints.

## Requirements

### Requirement 1: Implement Apalis SQLite Storage Setup

**User Story:** As a developer, I want the apalis SQLite storage to be properly initialized using apalis-sql with SQLite backend, so that the task queue can store and manage stepped jobs correctly.

#### Acceptance Criteria

1. WHEN the ApalisSqliteManager is created THEN it SHALL use apalis-sql SqliteStorage with proper configuration
2. WHEN setting up storage THEN it SHALL create the necessary database tables for stepped job management automatically
3. WHEN the storage is initialized THEN it SHALL support the apalis stepped task pattern with SQLite persistence
4. WHEN database connection fails THEN it SHALL provide clear error messages and retry mechanisms
5. WHEN using SQLite storage THEN it SHALL configure proper connection pooling and WAL mode for concurrent access

### Requirement 2: Implement Apalis Stepped Task Pattern

**User Story:** As a system administrator, I want transcription tasks to be processed through apalis stepped workflow, so that I can monitor progress and handle failures at each stage using apalis built-in capabilities.

#### Acceptance Criteria

1. WHEN a transcription task is submitted THEN it SHALL use apalis start_stepped() method to initiate the workflow
2. WHEN building steps THEN it SHALL use StepBuilder::new().step_fn() pattern for each processing stage
3. WHEN each step completes THEN it SHALL automatically pass data to the next step using apalis step transitions
4. WHEN a step fails THEN it SHALL use apalis built-in retry and error handling mechanisms
5. WHEN monitoring jobs THEN it SHALL leverage apalis job tracking to show current step and progress

### Requirement 3: Create Apalis-Compatible Stepped Job Types

**User Story:** As a developer, I want job types that are compatible with apalis stepped tasks requirements, so that transcription tasks can be properly serialized and processed through the step pipeline.

#### Acceptance Criteria

1. WHEN defining stepped job types THEN they SHALL implement apalis Job trait and be Serialize/Deserialize compatible
2. WHEN jobs are serialized THEN they SHALL be compatible with apalis SQLite storage format and stepped task requirements
3. WHEN jobs transition between steps THEN they SHALL maintain type safety and data integrity throughout the pipeline
4. WHEN job data is invalid THEN it SHALL fail gracefully with descriptive error messages using apalis error handling
5. WHEN defining step functions THEN they SHALL follow apalis step_fn signature requirements and return proper step transitions

### Requirement 4: Implement Apalis Worker Management

**User Story:** As a system operator, I want proper apalis worker management for processing stepped tasks, so that the system can handle concurrent transcription jobs efficiently using apalis WorkerBuilder.

#### Acceptance Criteria

1. WHEN workers are started THEN they SHALL use WorkerBuilder::new() with proper concurrency settings and SQLite backend
2. WHEN building workers THEN they SHALL use build_stepped() method with the defined step pipeline
3. WHEN multiple workers run THEN they SHALL coordinate through apalis SQLite storage without conflicts or duplicate processing
4. WHEN workers encounter errors THEN they SHALL use apalis built-in retry logic and error handling
5. WHEN the system shuts down THEN workers SHALL complete current jobs and stop cleanly using apalis graceful shutdown
6. WHEN configuring workers THEN they SHALL support apalis event handling with on_event() for monitoring

### Requirement 5: Add Apalis Job Status Monitoring and Management

**User Story:** As an API user, I want to monitor and manage apalis stepped jobs through HTTP endpoints under "/tasks" routes, so that I can track execution status (success, failure, queued, running) and manage jobs using apalis storage queries.

#### Acceptance Criteria

1. WHEN submitting a job via POST /tasks/transcribe THEN it SHALL use apalis start_stepped() and return a unique job ID for tracking
2. WHEN querying job status via GET /tasks/{job_id} THEN it SHALL query apalis SQLite storage to return job state (Pending, Running, Done, Failed, Killed)
3. WHEN job is queued THEN status SHALL show "Pending" with queue position and estimated start time
4. WHEN job is executing THEN status SHALL show "Running" with current step name and progress percentage
5. WHEN job completes successfully THEN status SHALL show "Done" with completion time and results
6. WHEN job fails THEN status SHALL show "Failed" with error details, retry count, and failed step information
7. WHEN cancelling a job via DELETE /tasks/{job_id} THEN it SHALL update apalis job status to "Killed" and stop processing
8. WHEN listing jobs via GET /tasks THEN it SHALL query apalis storage with filtering by status, priority, and pagination
9. WHEN accessing task endpoints THEN they SHALL be under "/tasks" route prefix as specified
10. WHEN monitoring jobs THEN it SHALL use apalis storage methods to retrieve job metadata and execution history
11. WHEN API returns job responses THEN they SHALL include proper HTTP status codes and structured JSON responses
12. WHEN invalid job IDs are requested THEN it SHALL return 404 Not Found with descriptive error messages

### Requirement 6: Integrate with Existing Transcription Logic

**User Story:** As a developer, I want the new apalis implementation to reuse existing transcription services, so that business logic remains consistent.

#### Acceptance Criteria

1. WHEN processing audio THEN it SHALL use existing AudioProcessor logic without modification
2. WHEN performing transcription THEN it SHALL use existing TranscriptionEngine with the same parameters
3. WHEN formatting results THEN it SHALL use existing response formatting to maintain API compatibility
4. WHEN handling errors THEN it SHALL maintain compatibility with existing error types and error handling patterns
5. WHEN integrating with existing services THEN it SHALL preserve all current functionality and behavior

### Requirement 7: Provide Configuration Management

**User Story:** As a system administrator, I want configurable settings for the apalis integration, so that I can tune performance and behavior for different environments.

#### Acceptance Criteria

1. WHEN configuring workers THEN it SHALL support concurrency settings and worker pool size configuration
2. WHEN configuring storage THEN it SHALL support database path, connection options, and connection pooling parameters
3. WHEN configuring jobs THEN it SHALL support timeout, retry settings, and cleanup policies
4. WHEN environment changes THEN configuration SHALL be updatable without code changes through environment variables
5. WHEN configuration is invalid THEN it SHALL provide clear validation errors and fallback to safe defaults
6. WHEN loading configuration THEN it SHALL support YAML files with environment variable overrides
7. WHEN configuration validation fails THEN it SHALL prevent service startup with descriptive error messages

### Requirement 8: Implement Apalis Job State Tracking

**User Story:** As a system administrator, I want comprehensive job state tracking using apalis built-in capabilities, so that I can monitor job lifecycle and troubleshoot issues effectively.

#### Acceptance Criteria

1. WHEN a job is submitted THEN it SHALL be stored in apalis SQLite storage with initial "Pending" state
2. WHEN worker picks up a job THEN apalis SHALL automatically update state to "Running" with timestamp
3. WHEN job completes successfully THEN apalis SHALL update state to "Done" with completion metadata
4. WHEN job encounters error THEN apalis SHALL update state to "Failed" with error details and retry information
5. WHEN job is cancelled THEN apalis SHALL update state to "Killed" and prevent further processing
6. WHEN querying job state THEN it SHALL use apalis storage queries to retrieve current status and metadata
7. WHEN job transitions between steps THEN it SHALL track step progress and current step name in job metadata
8. WHEN job exceeds retry limit THEN it SHALL be marked as permanently failed with detailed error history

### Requirement 9: Maintain Backward Compatibility

**User Story:** As an existing API user, I want the current synchronous endpoints to continue working, so that my applications don't break during the upgrade.

#### Acceptance Criteria

1. WHEN using existing /transcribe endpoint THEN it SHALL work exactly as before with same response format
2. WHEN new async /tasks endpoints are added THEN they SHALL not affect existing functionality or performance
3. WHEN apalis is disabled via configuration THEN the system SHALL fall back to existing worker pool seamlessly
4. WHEN migrating to async THEN it SHALL be optional and gradual with feature flags
5. WHEN both systems run concurrently THEN they SHALL not interfere with each other's operation
6. WHEN existing clients use synchronous endpoints THEN they SHALL continue to receive immediate responses without apalis job tracking
