# Document Parser

一个高性能的多格式文档解析服务，支持PDF、Word、Excel、PowerPoint等格式，具备GPU加速能力。

## 特性

- 🚀 **高性能解析**：支持MinerU和MarkItDown双引擎
- 🎯 **GPU加速**：通过sglang支持CUDA环境下的GPU加速
- 🔧 **零配置部署**：自动环境检测和依赖安装
- 📚 **多格式支持**：PDF、Word、Excel、PowerPoint、Markdown等
- 🌐 **HTTP API**：提供RESTful API接口
- 📊 **实时监控**：内置性能监控和健康检查

## 快速开始

### 1. 环境初始化
```bash
# 在当前目录初始化uv虚拟环境和依赖
document-parser uv-init
```

### 2. 启动服务
```bash
# 启动文档解析服务
document-parser server
```

服务将在 `http://localhost:8087` 启动，并自动激活虚拟环境。

## 文档

- 📖 **[用户使用手册](USER_MANUAL.md)** - 基本使用方法
- 🚀 **[CUDA环境配置指南](CUDA_SETUP_GUIDE.md)** - GPU加速配置
- 🔧 **[故障排除指南](TROUBLESHOOTING.md)** - 常见问题解决
- ⚡ **[GPU加速修复说明](GPU_ACCELERATION_FIX.md)** - GPU相关问题

## 系统要求

### 基本要求
- Rust 1.70+
- Python 3.8+
- uv (Python包管理器)

### GPU加速要求（可选）
- NVIDIA GPU (支持CUDA)
- CUDA Toolkit 11.8+
- 至少8GB GPU内存

## 支持的格式

| 格式 | 解析引擎 | 特性 |
|------|----------|------|
| PDF | MinerU | 专业PDF解析、图片提取、表格识别 |
| Word | MarkItDown | 文档结构保持、格式转换 |
| Excel | MarkItDown | 表格数据提取、格式保持 |
| PowerPoint | MarkItDown | 幻灯片内容提取、图片保存 |
| Markdown | 内置 | 实时解析、目录生成 |

## 配置说明

### 基本配置
```yaml
# 服务器配置
server:
  port: 8087
  host: "0.0.0.0"

# MinerU配置
mineru:
  backend: "vlm-sglang-engine"  # 启用GPU加速
  max_concurrent: 3
  quality_level: "Balanced"
```

### GPU加速配置
```yaml
mineru:
  backend: "vlm-sglang-engine"  # 使用sglang后端
  max_concurrent: 2              # GPU环境下建议降低并发数
  batch_size: 1
```

## 常用命令

```bash
# 环境管理
document-parser check              # 检查环境状态
document-parser uv-init            # 初始化环境
document-parser troubleshoot       # 故障排除指南

# 服务管理
document-parser server             # 启动服务
document-parser server --port 8088 # 指定端口

# 文件解析
document-parser parse --input file.pdf --output result.md --parser mineru
```

## API使用

### 解析文档
```bash
curl -X POST "http://localhost:8087/api/v1/documents/parse" \
  -H "Content-Type: multipart/form-data" \
  -F "file=@document.pdf" \
  -F "format=pdf"
```

### 获取解析状态
```bash
curl "http://localhost:8087/api/v1/documents/{task_id}/status"
```

## 性能优化

### GPU加速
1. 确保安装了 `sglang[all]`
2. 配置 `backend: "vlm-sglang-engine"`
3. 根据GPU内存调整并发参数
4. 监控GPU使用情况

### 并发控制
```yaml
mineru:
  max_concurrent: 2    # 根据系统性能调整
  batch_size: 1        # 小批次处理
  queue_size: 100      # 队列缓冲区大小
```

## 故障排除

### 常见问题
1. **虚拟环境未激活**：运行 `source ./venv/bin/activate`
2. **依赖安装失败**：运行 `document-parser uv-init`
3. **GPU加速不生效**：参考 [CUDA环境配置指南](CUDA_SETUP_GUIDE.md)
4. **权限问题**：检查目录权限和用户权限

### 获取帮助
```bash
# 详细故障排除指南
document-parser troubleshoot

# 环境状态检查
document-parser check

# 查看日志
tail -f logs/log.$(date +%Y-%m-%d)
```

## 开发

### 构建
```bash
cargo build --release
```

### 测试
```bash
cargo test
```

### 代码检查
```bash
cargo fmt
cargo clippy
```

## 许可证

本项目采用 MIT 许可证。

## 贡献

欢迎提交Issue和Pull Request！

---

**注意**：首次使用请运行 `document-parser uv-init` 初始化环境。如需GPU加速，请参考 [CUDA环境配置指南](CUDA_SETUP_GUIDE.md)。
