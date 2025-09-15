# MCP代理API

<cite>
**本文档中引用的文件**  
- [mcp_config.rs](file://mcp-proxy/src/model/mcp_config.rs)
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs)
- [mcp_check_status_model.rs](file://mcp-proxy/src/model/mcp_check_status_model.rs)
- [run_code_handler.rs](file://mcp-proxy/src/server/handlers/run_code_handler.rs)
- [sse_server.rs](file://mcp-proxy/src/server/handlers/sse_server.rs)
- [mcp_start_task.rs](file://mcp-proxy/src/server/task/mcp_start_task.rs)
- [http_result.rs](file://mcp-proxy/src/model/http_result.rs)
</cite>

## 目录
1. [简介](#简介)
2. [核心端点](#核心端点)
3. [MCP服务添加](#mcp服务添加)
4. [MCP服务状态查询](#mcp服务状态查询)
5. [执行代码指令](#执行代码指令)
6. [SSE实时输出流](#sse实时输出流)
7. [完整工作流示例](#完整工作流示例)
8. [错误码与响应结构](#错误码与响应结构)
9. [SSE连接管理](#sse连接管理)

## 简介
MCP代理服务提供了一套RESTful API，用于动态管理MCP（Model Control Protocol）服务实例的生命周期。该服务支持通过配置启动外部进程，并通过SSE（Server-Sent Events）协议实现实时双向通信。主要功能包括：添加新的MCP服务、查询服务状态、发送执行指令以及接收实时输出流。

**Section sources**
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs#L1-L90)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs#L1-L187)

## 核心端点
| 端点 | 方法 | 描述 |
|------|------|------|
| `/mcp/add` | POST | 添加新的MCP服务 |
| `/mcp/status/{mcpId}` | GET | 获取指定MCP服务的运行状态 |
| `/mcp/sse/proxy/{mcpId}/message` | POST | 向目标MCP服务发送执行指令 |
| `/mcp/sse/proxy/{mcpId}/sse` | GET | 建立SSE长连接接收实时输出 |

## MCP服务添加
### 请求信息
- **路径**: `POST /mcp/add`
- **内容类型**: `application/json`

### 请求体结构 (McpConfig)
```json
{
  "mcpId": "string",
  "mcpJsonConfig": "string",
  "mcpType": "persistent|oneShot",
  "mcpProtocol": "sse|stream"
}
```

| 字段 | 类型 | 必需 | 描述 |
|------|------|------|------|
| mcpId | string | 是 | MCP服务唯一标识符 |
| mcpJsonConfig | string | 是 | MCP服务的JSON配置，包含命令、参数和环境变量 |
| mcpType | string | 否 | 服务类型，默认为`oneShot`（一次性任务），可选`persistent`（持续运行） |
| mcpProtocol | string | 否 | 通信协议，默认为`sse` |

### 响应
- **成功**: `201 Created`
- **响应体**:
```json
{
  "code": "0000",
  "message": "成功",
  "data": {
    "mcp_id": "string",
    "sse_path": "string",
    "message_path": "string",
    "mcp_type": "persistent|oneShot"
  },
  "success": true
}
```

**Section sources**
- [mcp_config.rs](file://mcp-proxy/src/model/mcp_config.rs#L1-L73)
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs#L1-L90)
- [http_result.rs](file://mcp-proxy/src/model/http_result.rs#L1-L72)

## MCP服务状态查询
### 请求信息
- **路径**: `GET /mcp/status/{mcpId}`
- **内容类型**: `application/json`

### 请求参数
| 参数 | 位置 | 类型 | 必需 | 描述 |
|------|------|------|------|------|
| mcpId | 路径 | string | 是 | MCP服务唯一标识符 |
| mcpJsonConfig | 请求体 | string | 是 | MCP服务配置信息 |
| mcpType | 请求体 | string | 否 | 服务类型 |

### 响应结构
```json
{
  "ready": boolean,
  "status": "Ready|Pending|Error",
  "message": "string"
}
```

| 状态 | 描述 |
|------|------|
| Ready | 服务已就绪，可以接收指令 |
| Pending | 服务正在启动中 |
| Error | 服务启动失败 |

**Section sources**
- [mcp_check_status_model.rs](file://mcp-proxy/src/model/mcp_check_status_model.rs#L1-L100)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs#L1-L187)

## 执行代码指令
### 请求信息
- **路径**: `POST /mcp/sse/proxy/{mcpId}/message`
- **内容类型**: `application/json`

### 请求体结构
```json
{
  "code": "string",
  "json_param": {
    "key": "value"
  },
  "uid": "string",
  "engine_type": "js|ts|python"
}
```

| 字段 | 类型 | 必需 | 描述 |
|------|------|------|------|
| code | string | 是 | 要执行的代码 |
| json_param | object | 否 | 代码执行参数 |
| uid | string | 是 | 前端生成的随机UID，用于日志追踪 |
| engine_type | string | 是 | 执行引擎类型 |

### 响应
- **成功**: `200 OK`
- **响应体**:
```json
{
  "data": {},
  "success": boolean,
  "error": "string"
}
```

**Section sources**
- [run_code_handler.rs](file://mcp-proxy/src/server/handlers/run_code_handler.rs#L1-L84)

## SSE实时输出流
### 建立连接
- **路径**: `GET /mcp/sse/proxy/{mcpId}/sse`
- **协议**: SSE (Server-Sent Events)

### 事件类型
| 事件 | 触发条件 | 数据格式 |
|------|----------|----------|
| message | 收到服务输出 | `{"type":"message","data":"output text"}` |
| error | 发生错误 | `{"type":"error","data":"error message"}` |
| complete | 执行完成 | `{"type":"complete","data":"final result"}` |

### 响应头
```
Content-Type: text/event-stream
Cache-Control: no-cache
Connection: keep-alive
Transfer-Encoding: chunked
```

**Section sources**
- [sse_server.rs](file://mcp-proxy/src/server/handlers/sse_server.rs#L1-L95)
- [mcp_start_task.rs](file://mcp-proxy/src/server/task/mcp_start_task.rs#L1-L209)

## 完整工作流示例
### 1. 添加MCP服务
```bash
curl -X POST http://localhost:3000/mcp/add \
  -H "Content-Type: application/json" \
  -d '{
    "mcpId": "test-service",
    "mcpJsonConfig": "{\"command\":\"python\",\"args\":[\"-c\"],\"env\":{\"PYTHONPATH\":\"/app\"}}",
    "mcpType": "persistent",
    "mcpProtocol": "sse"
  }'
```

### 2. 查询服务状态
```bash
curl -X GET http://localhost:3000/mcp/status/test-service \
  -H "Content-Type: application/json" \
  -d '{
    "mcpId": "test-service",
    "mcpJsonConfig": "{\"command\":\"python\",\"args\":[\"-c\"],\"env\":{\"PYTHONPATH\":\"/app\"}}",
    "mcpType": "persistent"
  }'
```

### 3. 发送执行指令
```bash
curl -X POST http://localhost:3000/mcp/sse/proxy/test-service/message \
  -H "Content-Type: application/json" \
  -d '{
    "code": "print(\"Hello, World!\")",
    "json_param": {},
    "uid": "unique-request-id",
    "engine_type": "python"
  }'
```

### 4. 接收SSE响应
```bash
curl -X GET http://localhost:3000/mcp/sse/proxy/test-service/sse
```

## 错误码与响应结构
### 通用错误响应格式
```json
{
  "code": "string",
  "message": "string",
  "data": null,
  "success": false
}
```

### HTTP状态码
| 状态码 | 错误码 | 描述 |
|--------|--------|------|
| 400 | 400 | 请求路径无效或参数错误 |
| 404 | 0001 | MCP服务未找到 |
| 500 | 0002 | 启动MCP服务失败 |
| 503 | 0003 | 后端服务未就绪 |

### 错误码详情
| 错误码 | 描述 |
|--------|------|
| 0000 | 成功 |
| 0001 | 服务未找到 |
| 0002 | 启动失败 |
| 0003 | 服务未就绪 |
| 400 | 请求参数错误 |

**Section sources**
- [http_result.rs](file://mcp-proxy/src/model/http_result.rs#L1-L72)
- [mcp_add_handler.rs](file://mcp-proxy/src/server/handlers/mcp_add_handler.rs#L1-L90)
- [mcp_check_status_handler.rs](file://mcp-proxy/src/server/handlers/mcp_check_status_handler.rs#L1-L187)

## SSE连接管理
### 超时机制
- **连接超时**: 30秒
- **心跳间隔**: 15秒发送一次keep-alive消息
- **空闲超时**: 连接空闲5分钟后自动关闭

### 重连策略
1. **初始重连**: 立即重试
2. **指数退避**: 重试间隔按2^n增长（1s, 2s, 4s, 8s...）
3. **最大重试**: 最多重试10次
4. **随机抖动**: 添加±25%的随机时间避免雪崩

### 连接恢复
- 客户端应维护最后接收到的事件ID
- 重连时通过`Last-Event-ID`头传递最后事件ID
- 服务端根据ID恢复中断的流

**Section sources**
- [sse_server.rs](file://mcp-proxy/src/server/handlers/sse_server.rs#L1-L95)
- [mcp_start_task.rs](file://mcp-proxy/src/server/task/mcp_start_task.rs#L1-L209)