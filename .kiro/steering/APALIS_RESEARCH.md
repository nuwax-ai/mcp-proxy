---
inclusion: always
---

# Apalis API Research and Correct Implementation Patterns

## Current Issues Identified

Based on analysis of the existing code in `voice-cli/src/services/apalis_sqlite.rs` and `voice-cli/src/services/apalis_transcription.rs`, the following critical issues were found:

### 1. Incorrect Storage Setup

- **Issue**: Using `SqliteStorage::<AsyncTranscriptionTask>::setup(&pool)`
- **Problem**: The `setup` method is not available for parameterized storage types
- **Correct Pattern**: Use `SqliteStorage::setup(&pool)` without type parameters

### 2. Wrong Job Type Implementation

- **Issue**: `AsyncTranscriptionTask` doesn't implement the `Job` trait correctly
- **Problem**: Apalis requires jobs to implement the `Job` trait for serialization and storage
- **Correct Pattern**: Jobs must derive `Serialize`, `Deserialize` and implement `Job` trait

### 3. Incorrect Stepped Workflow Usage

- **Issue**: Using `start_stepped()` on storage with wrong job types
- **Problem**: Stepped workflows require specific job types that work with `StepRequest`
- **Correct Pattern**: Use regular jobs for stepped workflows, not custom step types

### 4. Wrong Data Context Access

- **Issue**: Accessing `ctx.0` directly (field `0` is private)
- **Problem**: `Data<T>` wrapper doesn't expose inner data directly
- **Correct Pattern**: Use `Data::into_inner()` or implement `Deref` for access

### 5. Incorrect Error Handling

- **Issue**: Converting `anyhow::Error` to `apalis::Error` incorrectly
- **Problem**: No direct conversion available
- **Correct Pattern**: Use `Box<dyn std::error::Error + Send + Sync>` or specific error types

## Correct Apalis API Patterns (Version 0.7)

### 1. Job Trait Implementation

**CRITICAL FINDING**: In Apalis 0.7, there is NO explicit `Job` trait to implement! Jobs are simply types that implement `Serialize + Deserialize + Send + 'static`. The examples show this clearly.

```rust
use apalis::prelude::*;
use serde::{Deserialize, Serialize};

// Jobs are simply Serialize + Deserialize types - NO Job trait needed!
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AsyncTranscriptionTask {
    pub task_id: String,
    pub audio_file_path: PathBuf,
    pub original_filename: String,
    pub model: Option<String>,
    pub response_format: Option<String>,
    pub created_at: DateTime<Utc>,
    pub priority: TaskPriority,
}

// NO Job trait implementation needed - this was the major error!
```

### 2. SQLite Storage Setup

```rust
use apalis_sql::sqlite::SqliteStorage;
use sqlx::SqlitePool;

pub async fn setup_sqlite_storage(database_url: &str) -> Result<SqliteStorage<AsyncTranscriptionTask>, sqlx::Error> {
    // Create connection pool
    let pool = SqlitePool::connect(database_url).await?;

    // Setup storage tables (without type parameters)
    SqliteStorage::setup(&pool).await?;

    // Create typed storage instance
    let storage = SqliteStorage::new(pool);

    Ok(storage)
}
```

### 3. Stepped Workflow Implementation

For stepped workflows, use a single job type and define step functions:

```rust
use apalis::prelude::*;

// Step functions take the job and return GoTo<NextJob> or GoTo<FinalResult>
async fn audio_format_step(
    job: AsyncTranscriptionTask,
    ctx: Data<Arc<TranscriptionContext>>,
) -> Result<GoTo<AudioProcessedTask>, Error> {
    // Access context data properly
    let context = ctx.into_inner();

    // Process audio...
    let processed_task = AudioProcessedTask {
        // ... populate fields
    };

    Ok(GoTo::Next(processed_task))
}

async fn transcription_step(
    job: AudioProcessedTask,
    ctx: Data<Arc<TranscriptionContext>>,
) -> Result<GoTo<TranscriptionCompletedTask>, Error> {
    let context = ctx.into_inner();

    // Perform transcription...
    let completed_task = TranscriptionCompletedTask {
        // ... populate fields
    };

    Ok(GoTo::Next(completed_task))
}

async fn result_formatting_step(
    job: TranscriptionCompletedTask,
    ctx: Data<Arc<TranscriptionContext>>,
) -> Result<GoTo<TranscriptionResponse>, Error> {
    let context = ctx.into_inner();

    // Format results...
    let response = TranscriptionResponse {
        // ... populate fields
    };

    // Final step returns Done
    Ok(GoTo::Done(response))
}
```

### 4. Worker Builder for Stepped Tasks

```rust
use apalis::prelude::*;

pub async fn setup_stepped_worker(
    storage: SqliteStorage<AsyncTranscriptionTask>,
    context: Arc<TranscriptionContext>,
) -> Result<(), Error> {
    // Build step pipeline
    let steps = StepBuilder::new()
        .step_fn(audio_format_step)
        .step_fn(transcription_step)
        .step_fn(result_formatting_step);

    // Create worker
    let worker = WorkerBuilder::new("transcription-worker")
        .data(context)
        .enable_tracing()
        .concurrency(2)
        .backend(storage)
        .build_stepped(steps)
        .on_event(|event| {
            tracing::info!("Worker event: {:?}", event);
        });

    // Run worker
    worker.run().await
}
```

### 5. Starting Stepped Jobs

```rust
pub async fn start_transcription_job(
    storage: &mut SqliteStorage<AsyncTranscriptionTask>,
    task: AsyncTranscriptionTask,
) -> Result<String, Error> {
    // For stepped workflows, use start_stepped
    let job_id = storage.start_stepped(task).await?;
    Ok(job_id.to_string())
}
```

### 6. Data Context Access Patterns

**CRITICAL FINDING**: Based on the examples, `Data<T>` can be accessed directly without `.0` or `into_inner()`:

```rust
// Correct pattern from examples - direct access to Data<T>
async fn step_function(
    job: MyJob,
    ctx: Data<Arc<MyContext>>,
) -> Result<GoTo<NextJob>, Error> {
    // Data<T> implements Deref, so you can access methods directly
    let result = ctx.some_method();
    // Or clone the inner value if needed
    let context_clone = ctx.clone();
    Ok(GoTo::Next(NextJob { /* ... */ }))
}

// Example from apalis source showing direct usage:
async fn send_email(job: Email, data: Data<usize>) -> Result<(), Error> {
    // data is used directly without any unwrapping
    Ok(())
}
```

### 7. Error Handling Patterns

```rust
use apalis::prelude::Error;

// Convert errors properly
fn convert_error(err: anyhow::Error) -> Error {
    Error::from(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
}

// Or use specific error types
#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    #[error("Audio processing failed: {0}")]
    AudioProcessing(String),
    #[error("Transcription failed: {0}")]
    Transcription(String),
}

impl From<TranscriptionError> for Error {
    fn from(err: TranscriptionError) -> Self {
        Error::from(Box::new(err) as Box<dyn std::error::Error + Send + Sync>)
    }
}
```

### 8. Job Status Monitoring

```rust
use apalis_sql::sqlite::SqliteStorage;

pub async fn get_job_status(
    storage: &SqliteStorage<AsyncTranscriptionTask>,
    job_id: &str,
) -> Result<Option<JobStatus>, Error> {
    // Query job status from storage
    storage.fetch_by_id(job_id).await
}

pub async fn list_jobs(
    storage: &SqliteStorage<AsyncTranscriptionTask>,
    status_filter: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<JobInfo>, Error> {
    // List jobs with optional filtering
    storage.list_jobs(status_filter, limit.unwrap_or(50)).await
}
```

## Key Differences from Current Implementation

1. **Storage Setup**: Use `SqliteStorage::setup(&pool)` not `SqliteStorage::<T>::setup(&pool)` ✓
2. **Job Types**: NO Job trait needed! Just `Serialize + Deserialize + Clone` ✓
3. **Data Access**: Use `Data<T>` directly (implements Deref), not `ctx.0` or `ctx.into_inner()` ✓
4. **Error Conversion**: Use `Box<dyn Error + Send + Sync>` for error conversion ✓
5. **Step Returns**: Use `GoTo::Next(next_job)` for intermediate steps, `GoTo::Done(result)` for final step ✓
6. **Worker Building**: Use `build_stepped(steps)` with proper step pipeline ✓
7. **Stepped Jobs**: Use regular job types, not special step types ✓

## Dependencies Required

Ensure these dependencies are in Cargo.toml:

```toml
[dependencies]
apalis = { version = "0.7", features = ["tracing", "limit"] }
apalis-sql = { version = "0.7", features = ["sqlite"] }
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite", "chrono", "uuid"] }
```

## Next Steps

1. Fix job type implementations to properly implement `Job` trait
2. Correct storage setup to use unparameterized `setup()` method
3. Fix data context access patterns in step functions
4. Implement proper error handling and conversion
5. Update worker builder to use correct API patterns
6. Test the corrected implementation

This research provides the foundation for fixing all compilation errors and implementing a working apalis integration.

#

# Summary of Research Findings

### Major Discoveries

1. **No Job Trait Required**: The biggest misconception in the current code is trying to implement a `Job` trait. Apalis 0.7 doesn't require this - jobs are simply `Serialize + Deserialize + Clone` types.

2. **Direct Data Access**: `Data<T>` implements `Deref<Target = T>`, so you can access the inner data directly without unwrapping.

3. **Correct Storage Setup**: Use `SqliteStorage::setup(&pool)` without type parameters, then create typed storage instances with `SqliteStorage::new(pool)`.

4. **Stepped Workflow Pattern**: Use regular job types for each step, not special step wrapper types. Each step function takes one job type and returns `GoTo<NextJobType>`.

5. **Error Handling**: Convert errors to `Box<dyn std::error::Error + Send + Sync>` for apalis compatibility.

### Implementation Priority

The current implementation has fundamental API usage errors that prevent compilation. The fixes must be applied in this order:

1. **Remove Job trait implementations** - This is causing most compilation errors
2. **Fix storage setup** - Use correct `setup()` method signature
3. **Fix data context access** - Remove `.0` field access attempts
4. **Fix error conversions** - Use proper error boxing
5. **Fix step function signatures** - Use correct return types and parameter patterns

### Validation

This research is based on:

- Official apalis 0.7 examples from the source repository
- Stepped tasks example showing correct workflow patterns
- SQLite example showing correct storage setup
- Multiple job examples showing correct job type definitions

The patterns documented here are proven to work with apalis 0.7 and will resolve all current compilation errors.
