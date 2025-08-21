# Requirements Document

## Introduction

This feature addresses a critical async runtime nesting issue in the document-parser service that causes panics when attempting to create new runtimes within existing tokio runtime contexts. The issue specifically occurs in the OSS client's upload_content method. Additionally, this feature includes creating a test interface to validate post-MinerU processing workflows using real parsing results, enabling faster testing by bypassing time-consuming PDF parsing steps.

## Requirements

### Requirement 1

**User Story:** As a developer, I want the OSS client to work properly in async environments, so that runtime nesting doesn't cause panics

#### Acceptance Criteria

1. WHEN the OSS client's upload_content method is called in an async context THEN the system SHALL execute normally without panicking
2. WHEN the system processes document upload requests THEN OSS upload operations SHALL complete successfully
3. IF the OSS client runs in an async environment THEN it SHALL use the current runtime instead of creating a new runtime
4. WHEN multiple concurrent upload operations occur THEN the system SHALL handle them without runtime conflicts

### Requirement 2

**User Story:** As a developer, I need a test interface to validate post-processing workflows using real MinerU parsing results, so that I can skip time-consuming PDF parsing steps and directly test downstream logic

#### Acceptance Criteria

1. WHEN the test interface is called with real MinerU parsing results (fixtures/upload_parse_test.md and fixtures/images/) THEN the system SHALL process the subsequent OSS upload workflow
2. WHEN using real parsing results for testing THEN the system SHALL validate Markdown processing, image uploading, and task status updates
3. WHEN testing is complete THEN the system SHALL return complete processing results including OSS links and task status
4. WHEN test files are missing or corrupted THEN the system SHALL return appropriate error messages

### Requirement 3

**User Story:** As a developer, I want a dedicated test endpoint `/api/v1/documents/test-post-mineru` to simulate MinerU parsing completion state, so that I can quickly test downstream processing workflows

#### Acceptance Criteria

1. WHEN the test endpoint POST `/api/v1/documents/test-post-mineru` is called with a task_id THEN the system SHALL use real parsing result files to simulate MinerU parsing completion state
2. WHEN simulating parsing completion THEN the system SHALL copy Markdown files and images from fixtures to corresponding temporary directories
3. WHEN downstream processing executes THEN the system SHALL properly handle real parsing results including image upload and Markdown processing
4. WHEN the task_id doesn't exist THEN the system SHALL return a 404 error with appropriate message
5. WHEN file copying fails THEN the system SHALL clean up partial operations and return error details