# 语音CLI API

<cite>
**本文档中引用的文件**  
- [handlers.rs](file://voice-cli/src/server/handlers.rs)
- [request.rs](file://voice-cli/src/models/request.rs)
- [stepped_task.rs](file://voice-cli/src/models/stepped_task.rs)
- [tts_service.rs](file://voice-cli/src/services/tts_service.rs)
- [mime_types.rs](file://voice-cli/src/utils/mime_types.rs)
- [tts.rs](file://voice-cli/src/models/tts.rs)
- [routes.rs](file://voice-cli/src/server/routes.rs)
</cite>

## 目录
1. [简介](#简介)
2. [核心接口](#核心接口)
3. [任务状态查询](#任务状态查询)
4. [SSE事件流](#sse事件流)
5. [音频输出格式](#音频输出格式)
6. [错误响应](#错误响应)
7. [使用示例](#使用示例)
8. [存储策略](#存储策略)

## 简介
语音CLI API提供文本转语音（TTS）功能，支持异步任务处理、实时进度通知和多种音频格式输出。系统采用任务队列架构，确保高并发下的稳定性和可扩展性。

## 核心接口

### 文本转语音接口 (POST /tts)
该接口用于提交文本转语音任务，接收JSON格式的请求体并返回任务对象。

**请求示例**
```json
{
  "text": "欢迎使用语音合成服务",
  "model": "zh-CN",
  "speed": 1.0,
  "volume": 1.0,
  "format": "mp3"
}
```

**请求参数**
- `text`: 要转换的文本内容（必填）
- `model`: 语音模型名称（可选，默认为系统默认模型）
- `speed`: 语速倍率（可选，0.5-2.0）
- `volume`: 音量级别（可选，0.0-2.0）
- `format`: 输出音频格式（可选，mp3/wav/ogg）

**响应结构**
返回SteppedTask任务对象，包含任务ID、当前状态和结果信息。

```json
{
  "task_id": "task-123456",
  "status": "pending",
  "result_url": null
}
```

**状态说明**
- `pending`: 任务已接收，等待处理
- `processing`: 正在处理中
- `completed`: 处理完成
- `failed`: 处理失败

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)
- [tts.rs](file://voice-cli/src/models/tts.rs#L0-L434)
- [routes.rs](file://voice-cli/src/server/routes.rs#L0-L200)

## 任务状态查询

### 获取任务状态 (GET /task/{taskId})
通过任务ID查询当前任务的状态和进度信息，支持长轮询模式。

**响应示例**
```json
{
  "task_id": "task-123456",
  "status": "processing",
  "message": "正在生成音频",
  "created_at": "2024-01-01T00:00:00Z",
  "updated_at": "2024-01-01T00:00:30Z"
}
```

**长轮询支持**
客户端可通过设置`timeout`参数实现长轮询，服务器将在状态变更时立即响应。

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)
- [stepped_task.rs](file://voice-cli/src/models/stepped_task.rs#L0-L416)

## SSE事件流

### 实时事件流 (GET /task/{taskId}/events)
通过SSE（Server-Sent Events）协议提供实时的合成进度事件通知。

**支持的事件类型**
- `started`: 任务开始处理
- `chunk_generated`: 音频片段生成
- `progress_update`: 进度更新
- `completed`: 任务完成
- `failed`: 任务失败

**事件流示例**
```
event: started
data: {"timestamp": "2024-01-01T00:00:30Z", "message": "开始语音合成"}

event: chunk_generated
data: {"chunk_id": 1, "duration_ms": 2000, "offset_ms": 0}

event: progress_update
data: {"progress": 0.5, "estimated_remaining_seconds": 30}

event: completed
data: {"result_url": "https://api.example.com/audio/task-123456.mp3", "duration_ms": 4500}
```

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)
- [stepped_task.rs](file://voice-cli/src/models/stepped_task.rs#L0-L416)

## 音频输出格式
系统支持多种音频格式输出，每种格式对应特定的MIME类型。

**支持的格式**
| 格式 | MIME类型 | 说明 |
|------|---------|------|
| MP3 | audio/mpeg | 高压缩比，广泛兼容 |
| WAV | audio/wav | 无损格式，文件较大 |
| OGG | audio/ogg | 开源格式，高压缩效率 |

**格式选择建议**
- **MP3**: 适用于大多数场景，平衡了音质和文件大小
- **WAV**: 适用于需要最高音质的专业场景
- **OGG**: 适用于Web应用，提供良好的压缩比

**Section sources**
- [mime_types.rs](file://voice-cli/src/utils/mime_types.rs#L0-L150)
- [request.rs](file://voice-cli/src/models/request.rs#L0-L434)

## 错误响应
API提供标准化的错误响应，包含HTTP状态码和详细的错误信息。

**常见错误类型**

### 400 Bad Request
请求参数无效或缺失必要字段。

```json
{
  "error": "Invalid request parameters",
  "details": "Text field is required and cannot be empty"
}
```

### 500 Internal Server Error
服务器内部错误，如模型加载失败。

```json
{
  "error": "Model loading failed",
  "details": "Failed to initialize TTS model 'zh-CN': Model file not found"
}
```

### 504 Gateway Timeout
请求处理超时。

```json
{
  "error": "Request timeout",
  "details": "TTS processing exceeded maximum allowed time of 300 seconds"
}
```

**错误处理建议**
- 客户端应实现重试机制，特别是对5xx错误
- 记录详细的错误日志用于问题排查
- 提供用户友好的错误提示

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)
- [stepped_task.rs](file://voice-cli/src/models/stepped_task.rs#L0-L416)

## 使用示例

### 完整流程示例
使用curl命令演示从提交任务到获取结果的完整流程。

**1. 提交TTS任务**
```bash
curl -X POST https://api.example.com/tts \
  -H "Content-Type: application/json" \
  -d '{
    "text": "欢迎使用语音合成服务",
    "model": "zh-CN",
    "speed": 1.0,
    "volume": 1.0,
    "format": "mp3"
  }'
```

**2. 查询任务状态**
```bash
curl https://api.example.com/task/task-123456
```

**3. 获取结果文件**
```bash
curl -O https://api.example.com/audio/task-123456.mp3
```

**4. 使用SSE监听进度**
```bash
curl https://api.example.com/task/task-123456/events
```

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)
- [routes.rs](file://voice-cli/src/server/routes.rs#L0-L200)

## 存储策略
系统采用分层存储策略管理生成的音频文件。

**存储生命周期**
- **临时存储**: 处理过程中的中间文件，任务完成后立即删除
- **结果存储**: 成功生成的音频文件，保留指定时间
- **归档存储**: 重要结果文件，长期保存

**TTL配置**
- 默认结果文件保留24小时
- 可通过配置文件调整保留时间
- 过期文件自动清理

**存储位置**
- 音频文件存储在`./data/audio`目录
- 数据库记录任务元数据
- 日志文件用于审计和监控

**Section sources**
- [tts_service.rs](file://voice-cli/src/services/tts_service.rs#L0-L300)
- [handlers.rs](file://voice-cli/src/server/handlers.rs#L0-L799)