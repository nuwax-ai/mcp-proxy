# CUDA环境配置和sglang GPU加速指南

## 概述

本指南专门针对需要GPU加速的用户，详细说明如何在支持CUDA的Linux服务器上配置sglang环境，确保MinerU能够使用GPU加速进行PDF解析。

## 前置条件

### 1. 硬件要求
- NVIDIA GPU（支持CUDA）
- 至少8GB GPU内存（推荐16GB+）
- 足够的系统内存（推荐32GB+）

### 2. 软件要求
- Linux操作系统（推荐Ubuntu 20.04+）
- NVIDIA驱动（版本450+）
- CUDA Toolkit（推荐11.8或12.x）
- Python 3.8+

## 环境检查

### 1. 检查NVIDIA驱动
```bash
# 检查驱动版本
nvidia-smi

# 预期输出示例：
# +-----------------------------------------------------------------------------+
# | NVIDIA-SMI 525.105.17   Driver Version: 525.105.17   CUDA Version: 12.0     |
# +-----------------------------------------------------------------------------+
```

### 2. 检查CUDA安装
```bash
# 检查CUDA版本
nvcc --version

# 预期输出示例：
# nvcc: NVIDIA (R) Cuda compiler driver
# Copyright (c) 2005-2023 NVIDIA Corporation
# Built on Wed_Nov_22_10:17:15_PST_2023
# Cuda compilation tools, release 12.3, V12.3.52
```

### 3. 检查GPU状态
```bash
# 查看GPU详细信息
nvidia-smi --query-gpu=index,name,memory.total,memory.free,compute_cap --format=csv

# 预期输出示例：
# 0, NVIDIA GeForce RTX 4090, 24576 MiB, 23552 MiB, 8.9
```

## 安装sglang

### 1. 激活虚拟环境
```bash
# 进入项目目录
cd /path/to/document-parser

# 激活虚拟环境
source ./venv/bin/activate

# 验证Python路径
which python
# 应该显示: /path/to/document-parser/venv/bin/python
```

### 2. 安装MinerU（包含兼容的sglang）
```bash
# 使用uv安装（推荐）
uv pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple

# 或者使用pip安装
pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple

# 安装过程可能需要几分钟，请耐心等待
```

**重要**：使用 `mineru[all]` 而不是直接安装 `sglang[all]`，确保版本兼容性。

### 3. 验证安装
```bash
# 检查sglang版本
python -c "import sglang; print('SGLang版本:', sglang.__version__)"

# 检查sglang server
python -m sglang.srt.server --help

# 检查CUDA支持
python -c "import torch; print('PyTorch版本:', torch.__version__); print('CUDA可用:', torch.cuda.is_available()); print('CUDA设备数:', torch.cuda.device_count())"
```

## 配置MinerU使用sglang

### 1. 修改配置文件
编辑 `config.yml` 文件：

```yaml
# MinerU配置
mineru:
  backend: "vlm-sglang-engine"  # 关键：启用sglang后端
  python_path: "./venv/bin/python"
  max_concurrent: 2              # GPU环境下建议降低并发数
  queue_size: 100
  batch_size: 1
  quality_level: "Balanced"
```

### 2. 或者通过环境变量
```bash
# 设置环境变量
export MINERU_BACKEND="vlm-sglang-engine"

# 启动服务
document-parser server
```

## 验证GPU加速是否生效

### 1. 启动服务并检查日志
```bash
# 启动服务
document-parser server

# 在另一个终端查看日志
tail -f logs/log.$(date +%Y-%m-%d)
```

查找以下关键信息：
```
INFO 虚拟环境已自动激活
INFO MinerU配置: backend=vlm-sglang-engine
DEBUG MinerU完整命令: .../mineru -p input.pdf -o output -b vlm-sglang-engine
```

### 2. 实时监控GPU使用
```bash
# 在另一个终端监控GPU
watch -n 1 nvidia-smi

# 或者使用更详细的监控
nvidia-smi dmon -s pucvmet -d 1
```

### 3. 测试PDF解析
上传一个PDF文件进行解析，观察：
- GPU内存使用是否增加
- GPU计算单元是否被占用
- 解析速度是否明显提升

### 4. 检查进程
```bash
# 查看MinerU进程
ps aux | grep mineru

# 查看GPU进程
nvidia-smi pmon -c 1
```

## 性能调优

### 1. 并发控制
根据GPU内存调整并发数：
```yaml
mineru:
  max_concurrent: 1    # 8GB GPU内存
  max_concurrent: 2    # 16GB GPU内存  
  max_concurrent: 4    # 24GB+ GPU内存
```

### 2. 批处理大小
```yaml
mineru:
  batch_size: 1        # 小批次，适合大模型
  batch_size: 2        # 中等批次
  batch_size: 4        # 大批次，适合小模型
```

### 3. 质量级别
```yaml
mineru:
  quality_level: "Fast"        # 快速模式，GPU占用低
  quality_level: "Balanced"    # 平衡模式（推荐）
  quality_level: "HighQuality" # 高质量模式，GPU占用高
```

## 故障排除

### 1. sglang导入失败
```bash
# 检查Python版本
python --version

# 重新安装sglang
pip uninstall sglang -y
pip install "sglang[all]"

# 检查依赖
pip list | grep sglang
```

### 2. CUDA不可用
```bash
# 检查PyTorch CUDA支持
python -c "import torch; print(torch.cuda.is_available())"

# 如果返回False，重新安装PyTorch
pip uninstall torch torchvision torchaudio -y
pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu118
```

### 3. GPU内存不足
```bash
# 检查GPU内存使用
nvidia-smi

# 降低并发数和批处理大小
# 关闭其他GPU进程
```

### 4. 版本兼容性问题
```bash
# 检查transformers版本
pip show transformers

# 安装兼容版本
pip install "transformers>=4.36.0,<4.40.0"

# 重新安装sglang
pip install "sglang[all]"
```

## 性能基准测试

### 1. 测试文件
使用不同大小的PDF文件测试性能：
- 小文件（<1MB）：测试启动时间
- 中等文件（1-10MB）：测试处理速度
- 大文件（>10MB）：测试内存使用

### 2. 性能指标
- **启动时间**：从命令执行到开始处理的时间
- **处理速度**：每秒处理的页数或字数
- **GPU利用率**：GPU计算单元和内存的使用率
- **内存使用**：GPU和系统内存的峰值使用

### 3. 对比测试
```bash
# 测试pipeline后端（CPU）
mineru -p test.pdf -o output -b pipeline

# 测试sglang后端（GPU）
mineru -p test.pdf -o output -b vlm-sglang-engine

# 对比处理时间和资源使用
```

## 监控和维护

### 1. 定期检查
```bash
# 检查GPU健康状态
nvidia-smi --query-gpu=health --format=csv

# 检查温度
nvidia-smi --query-gpu=temperature.gpu --format=csv

# 检查电源使用
nvidia-smi --query-gpu=power.draw --format=csv
```

### 2. 日志分析
```bash
# 分析性能日志
grep "processing_time" logs/log.* | awk '{print $NF}' | sort -n

# 分析错误日志
grep "ERROR" logs/log.* | tail -20
```

### 3. 性能优化
- 根据实际使用情况调整并发参数
- 监控GPU内存使用，避免OOM错误
- 定期清理临时文件和缓存

## 常见问题

### Q: 为什么GPU加速没有生效？
A: 检查以下几点：
1. sglang是否正确安装
2. 配置文件中的backend是否为"vlm-sglang-engine"
3. CUDA环境是否可用
4. GPU内存是否充足

### Q: 如何知道MinerU正在使用GPU？
A: 通过以下方式确认：
1. 查看nvidia-smi输出中的进程列表
2. 观察GPU内存使用是否增加
3. 检查日志中的命令参数
4. 对比CPU和GPU模式的性能差异

### Q: GPU内存不足怎么办？
A: 可以尝试：
1. 降低max_concurrent参数
2. 减小batch_size
3. 使用"Fast"质量级别
4. 关闭其他GPU进程

---

**注意**：本指南基于Linux环境编写，Windows用户可能需要调整部分命令。如有问题，请参考主用户手册或运行 `document-parser troubleshoot` 获取帮助。
