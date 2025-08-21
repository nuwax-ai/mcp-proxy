# Design Document

## Overview

This design document outlines the solution for fixing the async runtime nesting issue in the document-parser service and implementing a test interface for validating post-MinerU processing workflows.

The system currently uses a dual-engine architecture (MinerU + MarkItDown) supporting multi-format document parsing with complete OSS storage and structured processing workflows. The critical issue occurs during OSS upload operations where nested runtime creation causes panics in async contexts.

The solution involves converting the OSS client to fully async operations and creating a dedicated test endpoint that simulates MinerU completion using real parsing results from fixtures.

## Architecture

### Problem Analysis

The issue occurs in `oss-client/src/client.rs` within the `upload_content` method:

```rust
// Creates runtime and executes async operations
let rt = tokio::runtime::Runtime::new()
    .map_err(|e| OssError::sdk(format!("Failed to create runtime: {}", e)))?;

// Execute upload
match rt.block_on(self.client.put_object_from_file(&prefixed_key, &temp_path_string, builder)) {
    // ...
}
```

This code creates a new runtime within an existing async context, causing "Cannot start a runtime from within a runtime" panics.

### Solution Architecture

1. **OSS Client Async Conversion**

   - Convert `upload_content` method to async
   - Remove internal runtime creation, use current runtime directly
   - Update all callers to support async operations
   - Maintain backward compatibility where possible

2. **Test Interface Design**

   - Create new test endpoint `/api/v1/documents/test-post-mineru`
   - Simulate MinerU parsing completion state
   - Use real parsing result files for comprehensive testing
   - Support error scenarios and edge cases

3. **Concurrent Operation Support**
   - Ensure multiple simultaneous uploads work correctly
   - Implement proper error isolation between operations
   - Add comprehensive logging for debugging

## Components and Interfaces

### 1. OSS 客户端改造

#### Interface Changes

```rust
// Original synchronous method
pub fn upload_content(&self, content: &[u8], object_key: &str, content_type: Option<&str>) -> Result<String>

// New async method
pub async fn upload_content(&self, content: &[u8], object_key: &str, content_type: Option<&str>) -> Result<String>

// Additional methods to be converted
pub async fn upload_file(&self, file_path: &str, object_key: &str) -> Result<String>
pub async fn delete_object(&self, object_key: &str) -> Result<()>
```

#### Implementation Strategy

- Remove internal `tokio::runtime::Runtime::new()` and `block_on` calls
- Use `await` directly with async OSS SDK methods
- Maintain existing error handling patterns
- Add proper timeout handling for upload operations
- Implement retry logic for transient failures
- Preserve all existing functionality and error types

### 2. 测试接口设计

#### New Test Endpoint

```
POST /api/v1/documents/test-post-mineru
Content-Type: application/json

{
  "task_id": "existing-task-id"
}
```

#### Success Response Format

```json
{
  "success": true,
  "data": {
    "task_id": "task-id",
    "message": "Simulated MinerU parsing completion, starting downstream processing",
    "mineru_output_path": "temp/mineru/{task_id}",
    "markdown_file": "{task_id}.md",
    "images_count": 150,
    "processing_started": true
  }
}
```

#### Error Response Formats

```json
// Task not found
{
  "success": false,
  "error": {
    "code": "TASK_NOT_FOUND",
    "message": "Task with ID {task_id} not found"
  }
}

// File operation failed
{
  "success": false,
  "error": {
    "code": "FILE_OPERATION_FAILED",
    "message": "Failed to copy test files",
    "details": "Source file not found: fixtures/upload_parse_test.md"
  }
}
```

#### Core Functionality

The test interface simulates the complete MinerU workflow:

- Copy real MinerU parsing results (fixtures/upload_parse_test.md)
- Copy corresponding image files (fixtures/images/)
- Update task status to MinerU parsing completed
- Trigger downstream OSS upload and structured processing workflows
- Handle cleanup on partial failures
- Provide detailed logging for debugging

### 3. 文件处理流程

#### Test File Path Mapping

- Source Markdown file: `document-parser/fixtures/upload_parse_test.md`
- Source images directory: `document-parser/fixtures/images/`
- Target Markdown file: `document-parser/temp/mineru/{task_id}/{task_id}.md`
- Target images directory: `document-parser/temp/mineru/{task_id}/images/`
- Backup location: `document-parser/temp/mineru/{task_id}/backup/` (for rollback)

#### File Operation Sequence

1. Validate source files exist and are readable
2. Create target directory structure with proper permissions
3. Copy Markdown file with atomic operation (temp + rename)
4. Recursively copy image directory with progress tracking
5. Update task metadata with new file locations
6. Trigger downstream processing pipeline

## Data Models

### Test Request Model

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct TestPostMineruRequest {
    #[validate(length(min = 1, max = 100))]
    pub task_id: String,
}
```

### Test Response Model

```rust
#[derive(Debug, Serialize)]
pub struct TestPostMineruResponse {
    pub task_id: String,
    pub message: String,
    pub mineru_output_path: String,
    pub markdown_file: String,
    pub images_count: usize,
    pub processing_started: bool,
}
```

### Task Status Updates

```rust
// Update task status to MinerU parsing completed
task.status = TaskStatus::Processing;
task.mineru_completed = true;
task.mineru_output_path = Some(format!("temp/mineru/{}", task_id));
task.updated_at = Utc::now();
task.processing_metadata = Some(ProcessingMetadata {
    images_copied: images_count,
    markdown_size: markdown_file_size,
    test_mode: true,
});
```

## Error Handling

### OSS Client Error Handling

- Maintain existing error types and handling logic
- Ensure async errors propagate correctly through the call stack
- Add detailed error context with operation metadata
- Implement proper timeout handling with configurable durations
- Add retry logic for transient network failures
- Preserve error chain information for debugging

### Test Interface Error Handling

- Validate task_id exists and is in appropriate state
- Check source files exist and are readable
- Handle file copy operations with atomic transactions
- Implement cleanup for partial failures with rollback capability
- Provide detailed error messages with actionable information
- Log all operations for audit and debugging purposes

### Error Recovery Strategies

```rust
pub enum TestOperationError {
    TaskNotFound(String),
    InvalidTaskState { task_id: String, current_state: TaskStatus },
    SourceFileNotFound { path: String },
    FileOperationFailed { operation: String, path: String, cause: String },
    PartialFailure { completed_operations: Vec<String>, failed_operation: String },
}
```

## Testing Strategy

### 完整流程测试覆盖

基于 `upload_document` 接口的完整流程，需要测试以下关键步骤：

#### 1. 文件上传流程测试 (`process_multipart_upload_streaming`)

**测试用例 1.1: 正常文件上传流程**
```http
POST /api/v1/documents/upload
Content-Type: multipart/form-data
Content-Disposition: form-data; name="file"; filename="test.pdf"

# 验证步骤：
# - Multipart 数据解析
# - 文件名清理和验证
# - 文件扩展名验证
# - 临时文件创建 (create_temp_file_secure)
# - 流式写入 (stream_write_file_with_validation)
# - 文件格式检测 (detect_document_format_enhanced)
# - 进度监控和大小限制检查
```

**测试用例 1.2: 文件上传错误场景**
```http
# 测试文件过大
POST /api/v1/documents/upload?max_file_size=1024
Content-Type: multipart/form-data
# 上传超过限制的文件

# 测试无效文件格式
POST /api/v1/documents/upload
Content-Type: multipart/form-data
Content-Disposition: form-data; name="file"; filename="test.exe"

# 测试文件名安全性
POST /api/v1/documents/upload
Content-Type: multipart/form-data
Content-Disposition: form-data; name="file"; filename="../../../etc/passwd"
```

#### 2. 任务创建和管理流程测试

**测试用例 2.1: 任务创建流程**
```http
# 验证任务创建后的状态
GET /api/v1/tasks/{{task_id}}

# 预期响应：
{
  "success": true,
  "data": {
    "task": {
      "id": "{{task_id}}",
      "status": "Pending",
      "source_type": "Upload",
      "format": "PDF",
      "file_info": {
        "size": 1234567,
        "mime_type": "application/pdf"
      }
    }
  }
}
```

**测试用例 2.2: 文件信息更新验证**
```http
# 验证文件信息正确设置
GET /api/v1/tasks/{{task_id}}

# 验证字段：
# - file_size: 正确的文件大小
# - mime_type: 根据格式检测的MIME类型
# - original_filename: 清理后的文件名
```

#### 3. 异步解析流程测试

**测试用例 3.1: 跳过 MinerU 解析的完整流程**
```http
# 1. 上传文档（返回 task_id）
POST /api/v1/documents/upload
Content-Type: multipart/form-data
# 文件上传...

# 2. 验证任务初始状态
GET /api/v1/tasks/{{task_id}}
# 状态应为 Pending 或 Processing

# 3. 模拟 MinerU 解析完成（新增测试接口）
POST /api/v1/documents/test-post-mineru
{
  "task_id": "{{task_id}}"
}

# 4. 验证任务状态更新
GET /api/v1/tasks/{{task_id}}
# 状态应为 Processing (MinerU completed)

# 5. 验证文件复制结果
# - temp/mineru/{{task_id}}/{{task_id}}.md 存在
# - temp/mineru/{{task_id}}/images/ 目录存在
# - 图片文件正确复制

# 6. 验证后续 OSS 上传流程
GET /api/v1/tasks/{{task_id}}/result
# 验证 OSS 上传完成，任务状态为 Completed
```

#### 4. OSS 上传流程测试（异步运行时修复验证）

**测试用例 4.1: OSS 异步上传验证**
```http
# 验证 OSS 上传不会出现运行时嵌套错误
# 通过日志和任务状态确认上传成功

GET /api/v1/tasks/{{task_id}}/result
# 预期响应包含 OSS URL
{
  "success": true,
  "data": {
    "task_id": "{{task_id}}",
    "status": "Completed",
    "result": {
      "oss_url": "https://oss-url/path/to/file.md",
      "markdown_available": true
    }
  }
}
```

**测试用例 4.2: 并发 OSS 上传测试**
```http
# 同时触发多个任务的 OSS 上传
# 验证不会出现运行时冲突

# 任务 1
POST /api/v1/documents/test-post-mineru
{"task_id": "task-1"}

# 任务 2
POST /api/v1/documents/test-post-mineru  
{"task_id": "task-2"}

# 任务 3
POST /api/v1/documents/test-post-mineru
{"task_id": "task-3"}

# 验证所有任务都能正常完成
GET /api/v1/tasks/task-1/result
GET /api/v1/tasks/task-2/result  
GET /api/v1/tasks/task-3/result
```

#### 5. 错误处理和清理测试

**测试用例 5.1: 文件清理验证**
```http
# 测试上传失败时的文件清理
POST /api/v1/documents/upload
# 上传无效文件，验证临时文件被清理

# 测试任务创建失败时的清理
# 模拟任务服务不可用，验证上传的文件被清理
```

**测试用例 5.2: 部分失败恢复测试**
```http
# 测试文件复制部分失败的恢复
POST /api/v1/documents/test-post-mineru
{"task_id": "invalid-task-id"}

# 预期错误响应：
{
  "success": false,
  "error": {
    "code": "TASK_NOT_FOUND",
    "message": "Task with ID invalid-task-id not found"
  }
}
```

#### 6. 端到端集成测试

**测试用例 6.1: 完整工作流验证**
```http
# 完整流程：上传 → 解析 → OSS上传 → 结果获取
POST /api/v1/documents/upload
→ POST /api/v1/documents/test-post-mineru  
→ GET /api/v1/tasks/{{task_id}}/result
→ GET /api/v1/documents/{{task_id}}/markdown/download
→ GET /api/v1/documents/{{task_id}}/markdown/url
```

### 测试数据准备

#### 测试文件要求
- `fixtures/upload_parse_test.md`: 真实的 MinerU 解析结果
- `fixtures/images/`: 对应的图片文件目录
- 测试用的 PDF 文件（小文件，用于快速测试）
- 各种格式的测试文件（Word, Excel, PowerPoint等）

#### 测试环境配置
- OSS 测试配置（可以使用 mock 或测试环境）
- 临时目录权限设置
- 文件大小限制配置
- 超时时间配置

## Implementation Details

### 文件操作流程

1. 验证源文件存在性
2. 创建目标目录结构
3. 复制 Markdown 文件并重命名
4. 递归复制图片目录
5. 更新任务状态
6. 触发后续处理流程

### 异步处理链

```
测试接口调用 → 文件复制 → 任务状态更新 → 触发后续处理 → OSS 上传 → 完成
```

### 错误恢复机制

- 文件复制失败时清理已创建的目录
- 任务状态更新失败时回滚文件操作
- 提供详细的错误日志用于调试

## Implementation Focus

### 核心修复

1. **修复 OSS 客户端的异步运行时嵌套问题**
2. **创建简单的测试接口来跳过 MinerU 解析步骤**
3. **验证后续的 OSS 上传和任务完成流程**

### 简化实现

- 最小化代码变更，专注于解决核心问题
- 使用现有的错误处理和日志机制
- 复用现有的文件操作和任务管理逻辑
