# API参考文档

<cite>
**本文档中引用的文件**  
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs)
- [sse_server.rs](file://mcp-proxy/src/server/handlers/sse_server.rs)
- [mcp_config.rs](file://mcp-proxy/src/model/mcp_config.rs)
- [mcp_check_status_model.rs](file://mcp-proxy/src/model/mcp_check_status_model.rs)
- [http_result.rs](file://mcp-proxy/src/model/http_result.rs)
- [document_handler.rs](file://document-parser/src/handlers/document_handler.rs)
- [task_handler.rs](file://document-parser/src/handlers/task_handler.rs)
- [handlers.rs](file://voice-cli/src/server/handlers.rs)
</cite>

## 目录
1. [MCP代理服务API](#mcp代理服务api)
2. [文档解析服务API](#文档解析服务api)
3. [语音CLI服务API](#语音cli服务api)
4. [通用响应结构](#通用响应结构)
5. [错误响应示例](#错误响应示例)

## MCP代理服务API

### POST /mcp/add - 添加MCP服务
用于动态注册一个新的MCP（Model Control Protocol）服务实例。该接口接收MCP配置并启动对应的SSE或Stream代理服务。

**URL参数**  
无

**请求体（JSON）**  
```json
{
  "mcpJsonConfig": "string",     // MCP服务的JSON配置，包含命令、参数、环境变量等
  "mcpType": "oneShot|persistent" // MCP服务类型，默认为oneShot
}
```

**请求体结构说明**  
- `mcpJsonConfig`：符合McpServerCommandConfig结构的JSON字符串，包含：
  - `command`：要执行的命令（如Python脚本路径）
  - `args`：命令行参数数组
  - `env`：环境变量键值对
- `mcpType`：
  - `oneShot`：一次性任务，执行完成后自动清理
  - `persistent`：持久化服务，长期运行

**响应格式**  
成功时返回 `HttpResult<AddRouteResponseData>`：
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "mcp_id": "string",
    "sse_path": "/mcp/sse/proxy/{mcpId}/event",
    "message_path": "/mcp/sse/proxy/{mcpId}/message",
    "mcp_type": "oneShot|persistent"
  },
  "success": true
}
```

**HTTP状态码**  
- `200 OK`：服务成功注册，返回mcp_id和路径信息
- `400 Bad Request`：请求路径无效
- `500 Internal Server Error`：服务启动失败

**curl示例**  
```bash
curl -X POST http://localhost:3000/mcp/add \
  -H "Content-Type: application/json" \
  -d '{
    "mcpJsonConfig": "{\"command\":\"python\",\"args\":[\"test_python.py\"],\"env\":{\"PYTHONPATH\":\".\"}}",
    "mcpType": "persistent"
  }'
```

**Section sources**
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs#L1-L89)

### GET /mcp/status/{mcpId} - 检查MCP服务状态
检查指定mcp_id的服务是否已就绪，若未启动则根据配置自动启动。

**URL参数**  
- `mcpId`：MCP服务的唯一标识符

**请求体（JSON）**  
```json
{
  "mcpId": "string",
  "mcpJsonConfig": "string",
  "mcpType": "oneShot|persistent"
}
```

**响应格式**  
返回 `HttpResult<CheckMcpStatusResponseParams>`：
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "ready": false,
    "status": "Pending|Ready|Error",
    "message": "服务正在启动中..."
  },
  "success": true
}
```

**状态说明**  
- `Pending`：服务正在初始化或启动中
- `Ready`：服务已就绪，可接收消息
- `Error`：服务启动失败，包含错误信息

**HTTP状态码**  
- `200 OK`：状态检查成功，无论服务是否就绪
- `500 Internal Server Error`：内部处理错误

**curl示例**  
```bash
curl -X GET http://localhost:3000/mcp/status/abc123 \
  -H "Content-Type: application/json" \
  -d '{
    "mcpId": "abc123",
    "mcpJsonConfig": "{\"command\":\"python\",\"args\":[\"test_python.py\"]}",
    "mcpType": "persistent"
  }'
```

**Section sources**
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs#L1-L186)
- [mcp_check_status_model.rs](file://mcp-proxy/src/model/mcp_check_status_model.rs#L1-L100)

### POST /mcp/sse/proxy/{mcpId}/message - 发送SSE消息
向指定的MCP服务发送消息，触发其处理逻辑。

**URL参数**  
- `mcpId`：目标MCP服务的ID

**请求体**  
原始消息内容，格式由MCP服务定义（通常为JSON）

**响应格式**  
SSE流式响应，数据帧格式如下：
```
event: message
data: {"type":"update","content":"Processing..."}
id: 1

event: complete
data: {"result":"success","output":"Final result"}
id: 2
```

**事件类型**  
- `message`：中间状态更新或部分结果
- `error`：处理过程中发生错误
- `complete`：任务完成，包含最终结果

**HTTP状态码**  
- `200 OK`：成功建立SSE连接并开始流式传输
- `404 Not Found`：指定的mcpId不存在且无法创建
- `503 Service Unavailable`：后端MCP服务不可用

**curl示例**  
```bash
curl -X POST http://localhost:3000/mcp/sse/proxy/abc123/message \
  -H "Content-Type: text/plain" \
  -d "Hello, MCP!"
```

**Section sources**
- [sse_server.rs](file://mcp-proxy/src/server/handlers/sse_server.rs#L1-L94)

## 文档解析服务API

### POST /parse/document - 解析文档
上传文档并获取结构化解析结果。

**请求方法**  
POST

**Content-Type**  
`multipart/form-data`

**请求参数**  
- `file`：要解析的文档文件（支持txt、md、pdf等格式）
- `config`（可选）：解析配置JSON字符串

**响应格式**  
成功时返回 `HttpResult<StructuredDocument>`：
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "title": "文档标题",
    "sections": [
      {
        "heading": "第一章",
        "content": "章节内容...",
        "level": 1,
        "children": []
      }
    ],
    "metadata": {
      "author": "作者",
      "created_at": "2024-01-01T00:00:00Z"
    }
  },
  "success": true
}
```

**HTTP状态码**  
- `200 OK`：文档解析成功
- `400 Bad Request`：缺少文件或格式不支持
- `500 Internal Server Error`：解析过程中发生错误

**curl示例**  
```bash
curl -X POST http://localhost:8080/parse/document \
  -F "file=@sample_markdown.md" \
  -F "config={\"format\":\"markdown\"}" | jq
```

**Section sources**
- [document_handler.rs](file://document-parser/src/handlers/document_handler.rs)

## 语音CLI服务API

### POST /tts - 提交TTS任务
提交文本转语音任务。

**请求体（JSON）**  
```json
{
  "text": "要转换的文本",
  "voice": "voice_preset_name",
  "speed": 1.0
}
```

**响应格式**  
返回任务ID和初始状态：
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "taskId": "uuid-string",
    "status": "pending",
    "createdAt": "2024-01-01T00:00:00Z"
  }
}
```

### GET /tts/status/{taskId} - 轮询任务状态
通过SteppedTask机制轮询TTS任务状态。

**响应格式**  
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "taskId": "uuid-string",
    "status": "processing|completed|failed",
    "progress": 75,
    "resultUrl": "https://oss.example.com/audio.mp3"
  }
}
```

**轮询策略**  
客户端应使用指数退避策略进行轮询：
- 初始间隔：1秒
- 最大间隔：10秒
- 超时时间：5分钟

**curl示例**  
```bash
# 提交任务
TASK_ID=$(curl -X POST http://localhost:9000/tts \
  -H "Content-Type: application/json" \
  -d '{"text":"Hello World"}' | jq -r .data.taskId)

# 轮询状态
curl http://localhost:9000/tts/status/$TASK_ID
```

**Section sources**
- [handlers.rs](file://voice-cli/src/server/handlers.rs)

## 通用响应结构

所有API遵循统一的响应格式：

```json
{
  "code": "0000",
  "message": "成功",
  "data": {},
  "tid": "可选追踪ID",
  "success": true
}
```

**字段说明**  
- `code`：业务状态码，"0000"表示成功
- `message`：人类可读的消息
- `data`：具体响应数据，结构因接口而异
- `tid`：事务ID，用于问题追踪
- `success`：布尔值，等价于 code === "0000"

**Section sources**
- [http_result.rs](file://mcp-proxy/src/model/http_result.rs)

## 错误响应示例

### 400 Bad Request
```json
{
  "code": "400",
  "message": "无效的请求路径",
  "data": null,
  "success": false
}
```

### 503 Service Unavailable
```json
{
  "code": "0002",
  "message": "启动MCP服务失败: connection timeout",
  "data": null,
  "success": false
}
```

### 500 Internal Server Error
```json
{
  "code": "500",
  "message": "serde_json::to_string error",
  "data": null,
  "success": false
}
```

**Section sources**
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs#L75-L85)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs#L50-L60)