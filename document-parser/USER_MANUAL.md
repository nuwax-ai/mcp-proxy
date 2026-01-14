# Document Parser 用户使用手册

## 快速开始

### 系统依赖安装


```shell 

sudo apt update
sudo apt install --reinstall build-essential libc6-dev linux-libc-dev


sudo apt install gcc-multilib g++-multilib


```


### 1. 环境初始化
```bash
# 在当前目录初始化uv虚拟环境和依赖
document-parser uv-init
```

这个命令会：
- 检查并安装uv工具
- 创建虚拟环境 `./venv/`
- 安装MinerU和MarkItDown依赖
- 自动检测CUDA环境并安装相应版本

### 2. 启动服务
```bash
# 启动文档解析服务
document-parser server
```

服务启动后会自动：
- 激活虚拟环境
- 检查环境状态
- 启动HTTP服务器（默认端口8087）

## CUDA环境配置（GPU加速）

### 1. 检查CUDA环境
```bash
# 检查NVIDIA驱动和CUDA
nvidia-smi

# 检查CUDA版本
nvcc --version
```

### 2. 手动安装sglang（GPU加速必需）

**重要**：不要直接安装sglang，应该使用MinerU官方推荐的安装方式，确保版本兼容性。

```bash
# 激活虚拟环境
source ./venv/bin/activate

# 使用MinerU官方命令安装（推荐）
uv pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple

# 或者使用pip安装
pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple
```

**注意**：`mineru[all]` 会自动安装兼容的sglang版本，避免版本冲突问题。

### 3. 验证sglang安装
```bash
# 检查sglang版本
python -c "import sglang; print('SGLang版本:', sglang.__version__)"

# 检查sglang server是否可用
python -m sglang.srt.server --help
```

### 4. 验证CUDA编译器头文件查找
```bash
# 测试CUDA编译器是否能找到math.h头文件
nvcc -v -x cu - -o /dev/null <<< '#include <math.h>'

# 如果成功，应该显示编译信息
# 如果失败，会显示 "fatal error: math.h: 没有那个文件或目录"
```

## 配置MinerU使用sglang加速

### 1. 修改配置文件
编辑 `config.yml` 文件：

```yaml
# MinerU配置
mineru:
  backend: "vlm-sglang-engine"  # 启用sglang后端以支持GPU加速
  python_path: "./venv/bin/python"
  max_concurrent: 3
  queue_size: 100
  batch_size: 1
  quality_level: "Balanced"
```

### 2. 或者通过命令行指定
```bash
# 启动服务时指定后端
document-parser server --mineru-backend vlm-sglang-engine
```

## 验证MinerU是否使用sglang加速

### 1. 检查服务日志
启动服务后，查看日志中是否有以下信息：
```
INFO 虚拟环境已自动激活
INFO MinerU配置: backend=vlm-sglang-engine
```

### 2. 测试PDF解析
上传一个PDF文件进行解析，查看日志输出：
```
DEBUG MinerU完整命令: .../mineru -p input.pdf -o output -b vlm-sglang-engine
```

### 3. 检查GPU使用情况
在另一个终端中运行：
```bash
# 实时监控GPU使用
watch -n 1 nvidia-smi

# 或者使用htop查看进程
htop
```

如果看到MinerU进程占用GPU资源，说明sglang加速正常工作。

## 故障排除

### 1. sglang安装失败
```bash
# 检查Python版本（需要3.8+）
python --version

# 检查pip版本
pip --version

# 尝试升级pip
pip install --upgrade pip

# 重新安装sglang
pip uninstall sglang -y
pip install "sglang[all]"
```

### 2. 版本兼容性问题
如果遇到transformers版本兼容问题：
```bash
# 安装兼容的transformers版本
pip install "transformers>=4.36.0,<4.40.0"

# 重新安装MinerU（会自动安装兼容的sglang版本）
pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple
```

### 3. CUDA编译器头文件问题
如果遇到 `fatal error: math.h: 没有那个文件或目录` 错误：

```bash
# 验证CUDA编译器是否能找到math.h头文件
/usr/bin/nvcc -v -x cu - -o /dev/null <<< '#include <math.h>'

# 如果失败，安装缺失的开发包
sudo apt install -y libc6-dev libstdc++-13-dev

# 设置正确的CUDA环境变量
export CUDA_HOME=/usr
export PATH=/usr/lib/cuda/bin:$PATH
export LD_LIBRARY_PATH=/usr/lib/cuda/lib64:$LD_LIBRARY_PATH

# 清理FlashInfer缓存
rm -rf ~/.cache/flashinfer
```

**重要提示**：如果之前直接安装了sglang，建议先卸载再重新安装MinerU：
```bash
# 卸载可能不兼容的sglang版本
pip uninstall sglang -y

# 重新安装MinerU（包含兼容的sglang）
pip install -U "mineru[all]" -i https://mirrors.aliyun.com/pypi/simple
```

### 3. 虚拟环境问题
```bash
# 检查虚拟环境状态
document-parser check

# 重新初始化环境
rm -rf ./venv
document-parser uv-init
```

### 4. 权限问题
```bash
# 检查目录权限
ls -la

# 修改权限（如果需要）
chmod 755 .
chown $USER .
```

## 常用命令

### 环境诊断
```bash
# 验证CUDA编译器头文件查找
/usr/bin/nvcc -v -x cu - -o /dev/null <<< '#include <math.h>'

# 检查系统头文件是否存在
ls -la /usr/include/math.h
ls -la /usr/include/c++/13/cmath

# 检查CUDA安装路径
find /usr -name "cuda_runtime.h" 2>/dev/null
which nvcc
```

### 环境管理
```bash
# 检查环境状态
document-parser check

# 显示故障排除指南
document-parser troubleshoot

# 重新初始化环境
document-parser uv-init
```

### 服务管理
```bash
# 启动服务（默认端口8087）
document-parser server

# 指定端口启动
document-parser server --port 8088

# 指定配置文件
document-parser server --config custom_config.yml
```

### 文件解析
```bash
# 解析单个文件
document-parser parse --input input.pdf --output output.md --parser mineru
```

## 性能优化建议

### 1. GPU加速
- 确保安装了 `sglang[all]`
- 使用 `vlm-sglang-engine` 后端
- 监控GPU内存使用情况

### 2. 并发控制
根据服务器性能调整配置：
```yaml
mineru:
  max_concurrent: 2  # 根据GPU内存调整
  batch_size: 1      # 小批次处理
```

### 3. 超时设置
```yaml
document_parser:
  processing_timeout: 3600  # 60分钟超时
```

## 日志查看

### 1. 实时日志
```bash
# 查看当天日志
tail -f logs/log.$(date +%Y-%m-%d)

# 查看最新日志
tail -f logs/log.*
```

### 2. 日志级别
在 `config.yml` 中调整日志级别：
```yaml
log:
  level: "debug"  # 可选: debug, info, warn, error
```

## 联系支持

如果遇到问题：
1. 运行 `document-parser troubleshoot` 查看详细指南
2. 检查日志文件获取错误信息
3. 确保环境配置正确（Python版本、CUDA版本等）

---

**注意**：本手册基于当前版本编写，如有更新请参考最新文档。
