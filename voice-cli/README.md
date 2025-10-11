# Voice CLI - 语音转文字服务

高性能语音转文字 HTTP 服务，基于 Rust 构建，使用 Whisper 引擎提供准确的语音识别能力。

## 🚀 核心特性

### 语音处理能力
- **多格式支持**: MP3, WAV, FLAC, M4A, AAC, OGG 等主流音频格式
- **自动格式转换**: 基于 rs-voice-toolkit 的智能音频处理
- **Whisper 模型**: 支持 tiny/base/small/medium/large 系列模型
- **自动模型管理**: 按需下载和管理 Whisper 模型

### 部署模式
- **🏠 单节点部署**: 快速启动，适合小规模使用

### 高级功能
- **RESTful API**: 完整的 HTTP API 接口
- **实时监控**: 服务状态、健康检查
- **任务处理**: 高效的音频处理流水线

## 📋 系统要求

- **操作系统**: Linux/macOS/Windows
- **内存**: 最低 2GB，推荐 8GB+
- **存储**: 至少 5GB（用于模型存储）

## 🛠️ 快速安装

### 从源码构建
```bash
# 克隆项目
git clone https://github.com/your-org/mcp-proxy
cd mcp-proxy

# 构建 voice-cli
cargo build --release -p voice-cli --features=cuda

# 二进制文件位置
ls target/release/voice-cli
```

## 🏠 单节点部署

### 方式一：直接运行（最简单）

```bash
# 1. 切换到工作目录
mkdir -p /opt/voice-service
cd /opt/voice-service
cp /path/to/voice-cli ./

# 2. 启动服务（自动创建配置文件）
./voice-cli server run

# 3. 测试服务
curl -X POST http://localhost:8080/transcribe \
  -F "audio=@test.mp3" \
  -F "model=base"
```

### 方式二：后台运行（使用 nohup）

```bash
# 1. 启动后台服务
nohup ./voice-cli server run > server.log 2>&1 &

# 2. 检查进程状态
ps aux | grep voice-cli

# 3. 停止服务
pkill -f "voice-cli server run"

# 4. 查看日志
tail -f server.log
```

### 方式三：系统服务（推荐生产环境）

```bash
# 1. 创建配置文件
./voice-cli server init

# 2. 编辑配置文件（可选）
nano server-config.yml

# 3. 使用配置文件运行
./voice-cli server run --config server-config.yml
```

## 📝 配置文件

### 生成配置文件
```bash
# 生成默认配置文件
./voice-cli server init

# 生成到指定路径
./voice-cli server init --config /path/to/config.yml

# 强制覆盖现有文件
./voice-cli server init --force
```

### 配置文件示例 (server-config.yml)
```yaml
server:
  host: "0.0.0.0"
  port: 8080
  max_file_size: 268435456  # 256MB
  cors_enabled: true

whisper:
  default_model: "base"
  models_dir: "./models"
  auto_download: true
  supported_models:
    - "tiny"
    - "base"
    - "small"
    - "medium"
    - "large"
  audio_processing:
    sample_rate: 16000
    channels: 1
    bit_depth: 16
  workers:
    transcription_workers: 2
    channel_buffer_size: 100
    worker_timeout: 3600

logging:
  level: "info"
  log_dir: "./logs"
  max_file_size: "10MB"
  max_files: 10

daemon:
  pid_file: "./voice_cli.pid"
  log_file: "./logs/daemon.log"
  work_dir: "./work"
```

## 🔧 命令行使用

### 主要命令
```bash
# 初始化配置文件
voice-cli server init [--config <path>] [--force]

# 运行服务（前台模式）
voice-cli server run [--config <path>]

# 显示帮助信息
voice-cli --help
voice-cli server --help
```

### 环境变量配置
支持通过环境变量覆盖配置：
```bash
# HTTP 端口
VOICE_CLI_PORT=8081

# 日志级别
VOICE_CLI_LOG_LEVEL=debug

# 默认模型
VOICE_CLI_DEFAULT_MODEL=large

# 模型目录
VOICE_CLI_MODELS_DIR=/opt/models
```

## 🌐 HTTP API 接口

### 语音转录接口
```bash
POST /transcribe
Content-Type: multipart/form-data

参数:
- audio: 音频文件 (必填)
- model: 模型名称 (可选，默认使用配置的默认模型)
- language: 语言代码 (可选，如 "zh", "en")
- response_format: 响应格式 (可选，"json" 或 "text"，默认 "json")
```

### 健康检查接口
```bash
GET /health
```

### 示例请求
```bash
# 使用 curl
curl -X POST http://localhost:8080/transcribe \
  -F "audio=@speech.wav" \
  -F "model=base" \
  -F "language=zh"

# 响应示例
{
  "text": "你好，这是一个测试语音",
  "language": "zh",
  "duration": 5.2,
  "model": "base",
  "processing_time": 2.1
}
```

## 📊 监控和日志

### 日志文件
- `./logs/server.log` - 服务运行日志
- `./logs/daemon.log` - 后台服务日志

### 日志级别
支持以下日志级别：
- `trace` - 最详细的调试信息
- `debug` - 调试信息
- `info` - 一般信息（默认）
- `warn` - 警告信息
- `error` - 错误信息

## 🔍 故障排除

### 常见问题

1. **端口被占用**
   ```bash
   # 检查端口占用
   lsof -i :8080
   
   # 杀死占用进程
   kill -9 <PID>
   
   # 或者修改配置端口
   VOICE_CLI_PORT=8081 ./voice-cli server run
   ```

2. **模型下载失败**
   ```bash
   # 检查网络连接
   curl -I https://huggingface.co
   
   # 手动下载模型
   # 模型下载到 ./models/ggml-{model_name}.bin
   ```

3. **内存不足**
   ```bash
   # 使用较小的模型
   VOICE_CLI_DEFAULT_MODEL=tiny ./voice-cli server run
   
   # 减少工作线程
   VOICE_CLI_TRANSCRIPTION_WORKERS=1 ./voice-cli server run
   ```

### 调试模式
```bash
# 启用详细日志
RUST_LOG=debug ./voice-cli server run

# 查看实时日志
tail -f ./logs/server.log
```

## 📄 许可证

本项目采用 MIT 许可证。详见 [LICENSE](LICENSE) 文件。

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

## 📞 支持

- 提交 Issue: [GitHub Issues](https://github.com/your-org/mcp-proxy/issues)
- 文档: [项目 Wiki](https://github.com/your-org/mcp-proxy/wiki)