# TTS功能集成说明

本项目已成功集成文本转语音（TTS）功能，使用index-tts库作为核心语音合成引擎。

## 功能特性

### 同步接口
- **端点**: `POST /tts/sync`
- **功能**: 实时文本转语音，直接返回音频文件
- **适用场景**: 短文本、实时性要求高的场景

### 异步接口
- **端点**: `POST /api/v1/tasks/tts`
- **功能**: 提交TTS任务到队列，返回任务ID
- **任务查询**: 复用现有的 `/api/v1/tasks/{task_id}` 接口
- **结果获取**: 复用现有的 `/api/v1/tasks/{task_id}/result` 接口
- **适用场景**: 长文本、批量处理、后台任务

## 安装依赖

### 1. 安装uv包管理器
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

### 2. 安装Python依赖
```bash
cd voice-cli
uv sync
```

## 配置

### TTS配置选项
在配置文件中添加以下TTS相关配置：

```yaml
tts:
  python_path: null        # Python解释器路径（可选，默认自动查找）
  model_path: null         # TTS模型路径（可选）
  default_model: "default" # 默认语音模型
  supported_formats:       # 支持的音频格式
    - "mp3"
    - "wav"
  max_text_length: 5000    # 最大文本长度
  default_speed: 1.0       # 默认语速 (0.5-2.0)
  default_pitch: 0         # 默认音调 (-20到20)
  default_volume: 1.0      # 默认音量 (0.5-2.0)
  timeout_seconds: 300      # TTS任务超时时间
```

## API使用示例

### 同步TTS请求
```bash
curl -X POST "http://localhost:8080/tts/sync" \
  -H "Content-Type: application/json" \
  -d '{
    "text": "你好，世界！这是一个语音合成测试。",
    "speed": 1.0,
    "pitch": 0,
    "volume": 1.0,
    "format": "mp3"
  }'
```

### 异步TTS请求
```bash
curl -X POST "http://localhost:8080/api/v1/tasks/tts" \
  -H "Content-Type: application/json" \
  -d '{
    "text": "这是一个较长的文本，将通过异步处理转换为语音。",
    "speed": 1.2,
    "pitch": 5,
    "volume": 0.8,
    "format": "wav",
    "priority": "Normal"
  }'
```

### 查询任务状态
```bash
curl -X GET "http://localhost:8080/api/v1/tasks/{task_id}"
```

### 获取任务结果
```bash
curl -X GET "http://localhost:8080/api/v1/tasks/{task_id}/result"
```

## 请求参数说明

### TtsSyncRequest / TtsAsyncRequest
- `text` (string, 必需): 要合成的文本
- `model` (string, 可选): 语音模型名称
- `speed` (float, 可选): 语速，范围 0.5-2.0，默认1.0
- `pitch` (int, 可选): 音调，范围 -20到20，默认0
- `volume` (float, 可选): 音量，范围 0.5-2.0，默认1.0
- `format` (string, 可选): 输出格式，支持 "mp3", "wav"，默认"mp3"
- `priority` (string, 仅异步): 任务优先级 ("Low", "Normal", "High")

## 响应格式

### 同步响应
直接返回音频文件，包含适当的Content-Type头。

### 异步任务响应
```json
{
  "success": true,
  "data": {
    "task_id": "uuid-string",
    "message": "TTS任务已提交",
    "estimated_duration": 30
  }
}
```

## 任务状态

TTS任务支持以下状态：
- `Pending`: 任务已提交，等待处理
- `Processing`: 正在处理中
- `Completed`: 处理完成
- `Failed`: 处理失败
- `Cancelled`: 已取消

## 错误处理

### 常见错误码
- `400`: 请求参数错误
- `500`: 服务器内部错误

### 错误信息
- 文本长度超过限制
- 参数值超出范围
- TTS合成失败
- 文件操作错误

## 性能优化

1. **异步处理**: 长文本建议使用异步接口
2. **批量处理**: 可以提交多个异步任务
3. **缓存机制**: 相同文本可能会被缓存
4. **并发控制**: 通过配置控制最大并发任务数

## 扩展性

### 添加新的语音模型
1. 将模型文件放置在配置的model_path目录
2. 在请求中指定model参数

### 支持新的音频格式
1. 在配置的supported_formats中添加新格式
2. 确保index-tts支持该格式

## 故障排除

### 常见问题
1. **Python依赖缺失**: 运行 `uv sync` 安装依赖
2. **模型加载失败**: 检查model_path配置和模型文件
3. **权限问题**: 确保有写入data目录的权限
4. **内存不足**: 长文本可能需要更多内存

### 日志查看
```bash
tail -f logs/daemon.log
```

## 开发说明

### 代码结构
```
src/
├── models/
│   ├── tts.rs              # TTS相关数据模型
│   └── config.rs           # 配置结构
├── services/
│   ├── tts_service.rs      # TTS核心服务
│   └── tts_task_manager.rs # TTS任务管理器
└── server/
    ├── handlers.rs         # HTTP处理器
    └── routes.rs          # 路由配置
```

### 测试
```bash
cargo test tts
```

## 贡献

欢迎提交Issue和Pull Request来改进TTS功能。

## 许可证

遵循项目的开源许可证。