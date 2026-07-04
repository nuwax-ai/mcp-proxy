# Voice Toolkit - Rust 语音处理工具库

[![Crates.io](https://img.shields.io/crates/v/voice-toolkit.svg)](https://crates.io/crates/voice-toolkit)
[![Documentation](https://docs.rs/voice-toolkit/badge.svg)](https://docs.rs/voice-toolkit)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://rust-lang.org)

**Version: 0.15.0** - 统一的语音处理工具包

一个功能强大的 Rust 语音处理工具库，提供统一的 API 接口，支持语音转文本 (STT)、文本转语音 (TTS) 和音频处理功能。基于 OpenAI Whisper 模型提供高质量的语音识别，支持多种音频格式转换和实时语音处理。

## ✨ 主要特性

### 🎤 语音转文本 (STT)
- **Whisper 集成**: 使用 OpenAI Whisper 模型进行高质量语音识别
- **文件转录**: 支持多种音频格式的文件转录
- **流式转录**: 实时音频流处理，低延迟响应
- **VAD 集成**: 语音活动检测，智能识别语音片段
- **性能监控**: 内置性能基准测试和监控

### 🔊 文本转语音 (TTS)
- **Index-TTS 引擎**: 高质量的语音合成
- **可扩展架构**: 支持多种 TTS 引擎
- **灵活输出**: 支持内存缓冲区和文件输出
- **多格式支持**: 支持多种音频格式输出

### 🎵 音频处理
- **格式转换**: 支持 WAV、MP3、FLAC、M4A、OGG 等格式
- **音频重采样**: 使用 rubato 库进行高质量重采样
- **Whisper 兼容**: 自动转换为 Whisper 兼容格式（单声道、16kHz、16-bit PCM）
- **元数据提取**: 获取音频文件的详细信息
- **FFmpeg 集成**: 利用系统 FFmpeg 提供强大的格式支持

## 🚀 快速开始

### 基本依赖

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
voice-toolkit = { version = "0.15.0", features = ["stt", "tts", "audio"] }
```

### 系统要求

- **Rust**: 1.70 或更高版本
- **FFmpeg**: 用于音频处理
  - macOS: `brew install ffmpeg`
  - Ubuntu: `sudo apt-get install ffmpeg`
  - Windows: 使用 vcpkg 安装
- **Whisper 模型**: 需要下载 Whisper 模型文件（.bin 格式）

### 基本使用示例

#### 语音转文本

```rust
use voice_toolkit::transcribe_file_unified;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model_path = "models/ggml-base.bin";
    let audio_path = "audio/hello.wav";
    
    let result = transcribe_file_unified(model_path, audio_path).await?;
    println!("转录结果: {}", result.text);
    println!("处理时间: {:?}", result.processing_time);
    
    Ok(())
}
```

#### 音频格式转换

```rust
use voice_toolkit::audio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input_path = "audio/input.mp3";
    let output_path = "audio/output.wav";
    
    // 将 MP3 转换为 Whisper 兼容的 WAV 格式
    audio::convert_to_whisper_format(input_path, output_path).await?;
    println!("转换完成: {}", output_path);
    
    Ok(())
}
```

#### 文本转语音

```rust
use voice_toolkit::tts;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let text = "你好，世界！欢迎使用语音工具库。";
    let output_path = "output/hello.wav";
    
    // 使用 Index-TTS 生成语音
    tts::synthesize_text(text, output_path, None).await?;
    println!("语音合成完成: {}", output_path);
    
    Ok(())
}
```

## 📦 特性标志

工具包支持通过 Cargo 特性标志来选择功能：

```toml
[dependencies]
# 基本功能（STT + 音频处理）
voice-toolkit = { version = "0.15.0", features = ["stt", "audio"] }

# 完整功能（STT + TTS + 音频处理）
voice-toolkit = { version = "0.15.0", features = ["stt", "tts", "audio"] }

# 带流式处理
voice-toolkit = { version = "0.15.0", features = ["stt", "audio", "streaming"] }

# 带GPU加速
voice-toolkit = { version = "0.15.0", features = ["stt", "audio", "cuda"] }
```

### 可用特性：
- **`stt`**: 语音转文本功能（默认启用）
- **`tts`**: 文本转语音功能
- **`audio`**: 音频处理工具（默认启用）
- **`streaming`**: 实时流式转录（需要 `stt`）
- **`cuda`**: CUDA GPU 加速（需要 `stt`）
- **`vulkan`**: Vulkan GPU 加速（需要 `stt`）
- **`metal`**: Metal GPU 加速（需要 `stt`）

### 默认特性：
默认特性集包括 `stt` 和 `audio`，提供全面的语音处理能力。

## 🛠️ 安装和构建

### 依赖安装

#### FFmpeg 安装

**macOS:**
```bash
brew install ffmpeg
```

**Ubuntu/Debian:**
```bash
sudo apt-get update
sudo apt-get install ffmpeg
```

**Windows:**
使用 vcpkg 安装 FFmpeg，参考 [ez-ffmpeg 仓库](https://github.com/YeautyYE/ez-ffmpeg)

#### Whisper 模型下载

从 [Hugging Face](https://huggingface.co/ggerganov/whisper.cpp) 下载 Whisper 模型文件：

```bash
# 下载 tiny 模型（快速，适合测试）
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin

# 下载 base 模型（平衡性能和准确度）
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin

# 下载 large 模型（最高准确度）
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large.bin
```

### 构建项目

```bash
# 克隆仓库
git clone https://github.com/soddygo/rs-voice-toolkit.git
cd rs-voice-toolkit

# 构建所有组件
cargo build --release

# 运行测试
cargo test

# 生成文档
cargo doc --no-deps
```

## 📚 使用示例

### 基本转录示例

```bash
# 运行文件转录示例
cargo run -p stt --example transcribe_file -- models/ggml-base.bin samples/hello.wav

# 运行带 VAD 的转录
cargo run -p stt --example transcribe_with_vad -- models/ggml-base.bin samples/hello.wav
```

### 流式转录示例

```bash
# 启用 streaming 特性并运行流式转录
cargo run -p stt --features streaming --example streaming_transcribe -- models/ggml-base.bin samples/hello.wav
```

### 性能基准测试

```bash
# 运行性能基准测试
cargo run -p stt --example bench_transcribe -- models/ggml-base.bin samples/hello.wav 3
```

### TTS 示例

```bash
# 运行文本转语音示例
cargo run -p tts --example synthesize -- "你好，世界" out.wav
```

### 综合使用示例

```bash
# 运行综合示例
cargo run --example usage_examples

# 运行流式转录示例（需要 streaming 特性）
cargo run --example streaming_examples --features streaming
```

## 🏗️ 项目架构

### 核心模块

```
rs-voice-toolkit/
├── voice-toolkit/          # 统一接口库
│   ├── src/
│   │   ├── lib.rs         # 主要导出和文档
│   │   └── error.rs       # 统一错误处理
│   └── examples/
│       ├── usage_examples.rs           # 综合使用示例
│       └── streaming_examples.rs       # 流式转录示例
├── stt/                    # 语音转文本 (rs-voice-toolkit-stt)
│   ├── src/
│   │   ├── lib.rs         # STT 主要接口
│   │   ├── whisper.rs     # Whisper 模型封装
│   │   ├── streaming.rs   # 流式转录
│   │   ├── vad.rs         # 语音活动检测
│   │   └── error.rs       # STT 错误处理
│   └── examples/
│       ├── transcribe_file.rs      # 文件转录示例
│       ├── streaming_transcribe.rs  # 流式转录示例
│       ├── transcribe_with_vad.rs  # 带 VAD 的转录
│       └── bench_transcribe.rs     # 性能基准测试
├── audio/                  # 音频处理 (rs-voice-toolkit-audio)
│   ├── src/
│   │   ├── lib.rs         # 音频处理接口
│   │   ├── converter.rs   # 格式转换
│   │   ├── resampler.rs   # 音频重采样
│   │   └── utils.rs       # 工具函数
└── tts/                    # 文本转语音 (rs-voice-toolkit-tts)
    ├── src/
    │   ├── lib.rs         # TTS 主要接口
    │   ├── engine.rs      # TTS 引擎封装
    │   └── error.rs       # TTS 错误处理
    └── examples/
        └── synthesize.rs  # 语音合成示例
```

### 模块职责

#### voice-toolkit (统一接口)
- **职责**: 提供统一的 API 接口，整合所有子模块功能
- **特性**: 
  - 统一的错误处理机制
  - 便捷的函数封装
  - 完整的使用文档和示例

#### stt (语音转文本)
- **职责**: 基于 Whisper 模型的语音识别功能
- **特性**:
  - 支持多种 Whisper 模型（tiny、base、small、medium、large）
  - 文件转录和实时流式转录
  - 语音活动检测 (VAD) 集成
  - 多语言支持
  - 性能监控和基准测试

#### audio (音频处理)
- **职责**: 音频文件的格式转换、重采样和预处理
- **特性**:
  - 支持多种音频格式（WAV、MP3、FLAC、M4A、OGG）
  - 高质量音频重采样
  - Whisper 兼容格式自动转换
  - 音频元数据提取
  - FFmpeg 集成

#### tts (文本转语音)
- **职责**: 文本到语音的转换功能
- **特性**:
  - Index-TTS 引擎集成
  - 可扩展的引擎架构
  - 支持多种输出格式
  - 灵活的配置选项

## 🔧 错误处理

Voice Toolkit 使用统一的错误处理机制，所有函数都返回 `Result<T, Error>` 类型：

```rust
use voice_toolkit::{Error, Result};

fn process_audio() -> Result<()> {
    // 处理逻辑
    Ok(())
}

fn main() {
    match process_audio() {
        Ok(_) => println!("处理成功"),
        Err(Error::Audio(e)) => println!("音频处理错误: {}", e),
        Err(Error::Stt(e)) => println!("语音识别错误: {}", e),
        Err(Error::Tts(e)) => println!("语音合成错误: {}", e),
        Err(Error::Io(e)) => println!("IO错误: {}", e),
        Err(Error::Other(e)) => println!("其他错误: {}", e),
    }
}
```

## ⚡ 性能优化

### 模型加载优化
- 首次加载模型后复用实例，避免重复加载
- 对于长期运行的应用，保持模型实例在内存中

### GPU 加速
- 启用相应的 GPU 加速特性：
  - `cuda`: NVIDIA GPU 加速
  - `vulkan`: 跨平台 GPU 加速
  - `metal`: Apple Silicon GPU 加速

### 音频处理优化
- 使用 VAD (语音活动检测) 跳过静音部分
- 批量处理多个音频文件减少初始化开销
- 预转换音频格式为 Whisper 兼容格式

### 流式处理
- 对于实时应用，使用流式转录功能
- 配置适当的音频块大小平衡延迟和吞吐量

## 🧪 测试

### 运行测试

```bash
# 运行所有测试
cargo test

# 运行特定模块的测试
cargo test -p stt
cargo test -p audio
cargo test -p tts

# 运行集成测试
cargo test --test integration_tests
```

### 测试数据

项目包含测试用的音频文件和模型：

```bash
# 下载测试数据
./fixtures/get-fixtures.sh

# 运行测试示例
cargo run -p stt --example transcribe_file -- fixtures/models/ggml-tiny.bin fixtures/audio/jfk.wav
```

## 📄 许可证

本项目采用 MIT 或 Apache 2.0 双许可证。详情请参阅：

- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-APACHE](LICENSE-APACHE)

## 🤝 贡献

欢迎贡献代码！请遵循以下步骤：

1. Fork 本仓库
2. 创建特性分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

### 开发指南

- 代码风格：遵循 Rust 官方代码风格
- 文档：所有公共 API 都需要文档注释
- 测试：新功能需要相应的单元测试
- 错误处理：使用统一的错误类型

## 📞 支持

如果你在使用过程中遇到问题，请：

1. 查看 [文档](https://docs.rs/voice-toolkit)
2. 搜索已有的 [Issues](https://github.com/soddygo/rs-voice-toolkit/issues)
3. 创建新的 Issue 描述问题

## 🙏 致谢

- [OpenAI Whisper](https://github.com/openai/whisper) - 语音识别模型
- [whisper.cpp](https://github.com/ggerganov/whisper.cpp) - C/C++ 实现
- [Index-TTS](https://github.com/open-mmlab/Index-TTS) - 语音合成引擎
- [FFmpeg](https://ffmpeg.org/) - 音频处理工具
- [Rubato](https://github.com/HEnquist/rubato) - 音频重采样库

## 📊 性能基准

在配备 Apple M1 Pro 的 MacBook Pro 上的性能数据：

| 模型 | 音频长度 | 处理时间 | RTF | 内存使用 |
|------|----------|----------|-----|----------|
| tiny | 10s | 0.8s | 0.08 | 200MB |
| base | 10s | 2.1s | 0.21 | 400MB |
| small | 10s | 6.2s | 0.62 | 800MB |
| medium | 10s | 18.5s | 1.85 | 1.5GB |
| large | 10s | 42.3s | 4.23 | 2.8GB |

*RTF (Real-Time Factor) = 处理时间 / 音频时长，值小于 1 表示能实时处理*


