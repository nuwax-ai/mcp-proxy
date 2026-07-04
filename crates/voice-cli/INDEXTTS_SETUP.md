# IndexTTS 安装和配置指南

## 概述

本指南介绍如何安装和配置 IndexTTS 环境，用于 voice-cli 项目的文本转语音功能。IndexTTS 是一个基于 GPT 的高质量文本转语音系统，支持中文和英文。

## 系统要求

- **操作系统**: macOS 或 Linux
- **Python**: 3.10.x (必须使用 3.10 版本)
- **内存**: 至少 8GB RAM
- **存储**: 至少 5GB 可用空间（用于模型文件）
- **网络**: 稳定的互联网连接（用于下载模型）
- **包管理器**: uv (推荐) 或 pip

## 快速安装

### 1. 自动安装（推荐）

运行自动安装脚本：

```bash
cd /path/to/voice-cli
./install_indextts.sh
```

这个脚本会自动处理：
- 系统要求检查
- Python 3.10 安装
- uv 包管理器安装
- 虚拟环境创建
- PyTorch 安装
- IndexTTS 从源码安装
- 模型文件下载
- 参考语音文件创建
- 环境测试

### 2. 手动安装

如果自动安装失败，可以按照以下步骤手动安装：

#### 2.1 安装系统依赖

**macOS:**
```bash
brew install python@3.10 ffmpeg git
```

**Linux (Ubuntu/Debian):**
```bash
sudo apt-get update
sudo apt-get install python3.10 python3.10-venv ffmpeg git
```

#### 2.2 安装 uv 包管理器
```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

#### 2.3 创建虚拟环境
```bash
cd /path/to/voice-cli
python3.10 -m venv .venv
source .venv/bin/activate
```

#### 2.4 安装 PyTorch
```bash
uv add torch torchaudio --index-url https://download.pytorch.org/whl/cpu
```

#### 2.5 安装 IndexTTS
```bash
git clone https://github.com/index-tts/index-tts.git /tmp/index-tts
cd /tmp/index-tts
pip install -e .
cd /path/to/voice-cli
```

#### 2.6 下载模型
```bash
uv add huggingface-hub
python -c "
from huggingface_hub import snapshot_download
snapshot_download(
    repo_id='IndexTeam/IndexTTS-1.5',
    local_dir='checkpoints'
)
"
```

#### 2.7 创建参考语音文件
```bash
# macOS
say -v "Alex" "This is a reference voice for IndexTTS" -o reference_voice.aiff
ffmpeg -y -i reference_voice.aiff reference_voice.wav
rm -f reference_voice.aiff

# Linux
# 使用录音工具录制参考语音，或使用现有的 WAV 文件
```

## 配置 voice-cli

### 1. 更新 pyproject.toml

确保 `pyproject.toml` 文件包含以下内容：

```toml
[project]
name = "voice-cli-tts"
version = "0.1.0"
description = "TTS dependencies for voice-cli"
requires-python = ">=3.10,<3.11"
dependencies = [
    "torch>=2.8",
    "torchaudio>=2.8",
    "numpy>=1.19.0,<2.0.0",
    "soundfile>=0.12",
    "huggingface-hub>=0.34.4",
]

[tool.uv]
dev-dependencies = []

[build-system]
requires = ["hatchling"]
build-backend = "hatchling.build"
```

### 2. 使用 uv 管理依赖

```bash
# 激活虚拟环境
source .venv/bin/activate

# 同步依赖
uv sync

# 添加新依赖
uv add <package_name>

# 移除依赖
uv remove <package_name>
```

### 3. 参考语音文件

IndexTTS 需要一个参考语音文件来生成语音。参考语音文件应该是：
- 格式：WAV
- 时长：3-30 秒
- 质量：清晰、无噪音
- 内容：任何语音内容

## 测试安装

### 1. 运行测试脚本
```bash
source .venv/bin/activate
python test_indextts.py
```

### 2. 手动测试 IndexTTS
```bash
source .venv/bin/activate
indextts "你好，这是一个测试" \
  --voice reference_voice.wav \
  --output_path test_output.wav \
  --model_dir checkpoints \
  --config checkpoints/config.yaml \
  --force
```

### 3. 测试 TTS 服务
```bash
source .venv/bin/activate
python tts_service.py "你好世界" --output tts_test.wav
```

### 4. 测试 voice-cli 服务器
```bash
# 重新编译 voice-cli
cargo install --path voice-cli --features=metal

# 启动服务器
voice-cli server run

# 在另一个终端测试 TTS 接口
curl --location --request POST 'http://127.0.0.1:8087/tts/sync' \
--header 'Content-Type: application/json' \
--data-raw '{
    "text": "测试成功"
}' --max-time 60
```

## 部署和使用

### 1. 日常使用

```bash
# 激活虚拟环境
source .venv/bin/activate

# 启动服务器
voice-cli server run

# 测试 TTS 接口
curl -X POST 'http://127.0.0.1:8087/tts/sync' \
  -H 'Content-Type: application/json' \
  -d '{"text": "你好世界"}' \
  --max-time 60
```

### 2. 使用 uv 管理环境

```bash
# 更新所有依赖
uv sync

# 清理缓存
uv cache clean

# 查看环境信息
uv pip list
```

### 3. 性能监控

IndexTTS 处理时间通常为 15-30 秒，具体取决于：
- 文本长度
- CPU 性能
- 内存可用性

## 故障排除

### 1. 常见问题

**问题：Python 版本不兼容**
```bash
# 解决方案：确保使用 Python 3.10.x
python3.10 --version
# 应该显示 Python 3.10.x
```

**问题：模型文件缺失**
```bash
# 解决方案：手动下载模型文件
source .venv/bin/activate
python -c "
from huggingface_hub import hf_hub_download
hf_hub_download(repo_id='IndexTeam/IndexTTS-1.5', filename='gpt.pth', local_dir='checkpoints')
hf_hub_download(repo_id='IndexTeam/IndexTTS-1.5', filename='bigvgan_generator.pth', local_dir='checkpoints')
"
```

**问题：FFmpeg 未安装**
```bash
# 解决方案：
macOS: brew install ffmpeg
Linux: sudo apt-get install ffmpeg
```

**问题：uv 命令不存在**
```bash
# 解决方案：重新安装 uv
curl -LsSf https://astral.sh/uv/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

**问题：IndexTTS 导入失败**
```bash
# 解决方案：重新安装 IndexTTS
source .venv/bin/activate
cd /tmp/index-tts
git pull
pip install -e .
```

### 2. 日志调试

启用详细日志：
```bash
export RUST_LOG=debug
voice-cli server run
```

检查 Python 环境：
```bash
source .venv/bin/activate
python -c "import indextts; print('IndexTTS 导入成功')"
```

检查模型文件：
```bash
ls -la checkpoints/
# 应该包含：config.yaml, gpt.pth, bigvgan_generator.pth, dvae.pth 等
```

### 3. 网络问题

如果下载模型失败，可以：
1. 使用代理：`export HF_ENDPOINT=https://hf-mirror.com`
2. 手动下载文件到 checkpoints 目录
3. 使用 wget 下载：
```bash
wget https://huggingface.co/IndexTeam/IndexTTS-1.5/resolve/main/gpt.pth -P checkpoints
```

### 4. 性能优化

- **CPU 优化**：确保使用最新版本的 PyTorch
- **内存优化**：如果内存不足，关闭其他占用内存的应用
- **并发优化**：调整 voice-cli 配置中的并发设置

## 高级配置

### 1. 使用不同的模型

IndexTTS 提供多个模型版本：
- `IndexTeam/IndexTTS`：基础版本
- `IndexTeam/IndexTTS-1.5`：最新版本（推荐）

### 2. 自定义配置

创建自定义配置文件：
```yaml
# custom_config.yaml
model:
  path: "checkpoints/gpt.pth"
  device: "cpu"  # 或 "cuda" 如果有 GPU

audio:
  sample_rate: 22050
  format: "wav"

synthesis:
  speed: 1.0
  pitch: 0.0
  volume: 1.0
```

### 3. 批量处理

使用 voice-cli 的异步任务接口进行批量处理：
```bash
curl --location --request POST 'http://127.0.0.1:8087/api/v1/tasks/tts' \
--header 'Content-Type: application/json' \
--data-raw '{
    "text": "批量处理文本",
    "format": "wav",
    "priority": "normal"
}'
```

## 维护和更新

### 1. 定期更新

```bash
# 更新 IndexTTS
source .venv/bin/activate
cd /tmp/index-tts
git pull
pip install -e .

# 更新依赖
uv sync

# 更新模型
python -c "
from huggingface_hub import snapshot_download
snapshot_download(
    repo_id='IndexTeam/IndexTTS-1.5',
    local_dir='checkpoints',
    allow_patterns='*.pth,*.yaml,*.model'
)
"
```

### 2. 清理维护

```bash
# 清理下载缓存
rm -rf ~/.cache/huggingface
uv cache clean

# 清理日志
rm -rf logs/*

# 清理临时文件
rm -rf *.tmp *.temp
```

### 3. 备份重要文件

```bash
# 备份模型文件
tar -czf index_tts_models_backup.tar.gz checkpoints/

# 备份配置文件
cp pyproject.toml pyproject.toml.backup
cp reference_voice.wav reference_voice.wav.backup
```

## 许可证

- IndexTTS：遵循其开源许可证
- 模型文件：遵循各自的许可证条款
- voice-cli：项目许可证

## 支持

- **IndexTTS 官方文档**：https://github.com/index-tts/index-tts
- **uv 包管理器**：https://docs.astral.sh/uv/
- **问题反馈**：GitHub Issues
- **社区支持**：Discord、QQ群

## 最佳实践

1. **环境管理**：始终使用虚拟环境和 uv 管理依赖
2. **模型管理**：定期备份重要的模型文件
3. **性能监控**：关注 TTS 处理时间和内存使用
4. **错误处理**：实现适当的重试机制和错误处理
5. **测试验证**：定期测试 TTS 功能确保正常工作

## 注意事项

- IndexTTS 首次运行可能需要较长时间（15-30秒）
- 确保有足够的内存和存储空间
- 网络连接对于模型下载很重要
- 建议定期备份模型文件和配置